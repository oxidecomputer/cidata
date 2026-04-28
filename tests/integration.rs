// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![allow(clippy::explicit_deref_methods)]

use std::fmt::Debug;
use std::io::Cursor;
use std::io::ErrorKind;
use std::io::Read;
use std::ops::Deref;
use std::ops::Not;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use atomicwrites::AtomicFile;
use atomicwrites::OverwriteBehavior;
use cidata::Cidata;
use fatfs::FatType;
use fatfs::FsOptions;
use flate2::Compression;
use flate2::write::GzEncoder;
use proptest::collection::vec;
use proptest::prelude::Just;
use proptest::prelude::any;
use proptest::test_runner::Config;
use proptest::test_runner::TestRunner;
use tar::Header;
use test_strategy::Arbitrary;
use test_strategy::proptest;
use uuid::Uuid;
use xshell::Cmd;
use xshell::Shell;
use xshell::cmd;

#[derive(Debug, Arbitrary)]
struct TestInput {
    meta_data: Option<TestData>,
    network_config: Option<TestData>,
    user_data: Option<TestData>,
    vendor_data: Option<TestData>,
}

impl TestInput {
    fn as_cidata(&self) -> Cidata<'_> {
        Cidata {
            meta_data: self.meta_data.as_deref(),
            network_config: self.network_config.as_deref(),
            user_data: self.user_data.as_deref(),
            vendor_data: self.vendor_data.as_deref(),
        }
    }
}

// These values are set so that we're likely to get generated
// filesystems that contain large enough (zeroed) files to force larger
// cluster sizes, then also have a file with random data split over that
// cluster size.
const SMALL_LIMIT: usize = 8 * 1024;
const LARGE_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Arbitrary)]
enum TestData {
    Small(#[strategy(vec(any::<u8>(), 0..=SMALL_LIMIT))] Vec<u8>),
    Large(#[strategy(vec(Just(0), SMALL_LIMIT..=LARGE_LIMIT))] Vec<u8>),
}

impl Debug for TestData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Small(v) => f.debug_tuple("Small").field(&v.len()).finish(),
            Self::Large(v) => f.debug_tuple("Large").field(&v.len()).finish(),
        }
    }
}

impl Deref for TestData {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match &self {
            TestData::Small(x) | TestData::Large(x) => x,
        }
    }
}

trait FileSystem {
    type File<'a>: Read
    where
        Self: 'a;

    fn open(&self, path: &str) -> std::io::Result<Self::File<'_>>;
}

impl FileSystem for fatfs::FileSystem<Cursor<Vec<u8>>> {
    type File<'a> = fatfs::File<'a, Cursor<Vec<u8>>>;

    fn open(&self, path: &str) -> std::io::Result<Self::File<'_>> {
        self.root_dir().open_file(path)
    }
}

struct MountedFileSystem(PathBuf);

impl FileSystem for MountedFileSystem {
    type File<'a> = std::fs::File;

    fn open(&self, path: &str) -> std::io::Result<Self::File<'_>> {
        std::fs::File::open(self.0.join(path))
    }
}

fn check_file_system(cidata: &Cidata<'_>, file_system: &impl FileSystem) {
    for (file_name, data) in [
        ("meta-data", cidata.meta_data),
        ("network-config", cidata.network_config),
        ("user-data", cidata.user_data),
        ("vendor-data", cidata.vendor_data),
    ] {
        match (data, file_system.open(file_name)) {
            (Some(expected_data), Ok(mut file)) => {
                let mut file_data = Vec::with_capacity(expected_data.len());
                file.read_to_end(&mut file_data).unwrap();
                assert!(
                    expected_data == file_data,
                    "{file_name} data incorrect"
                );
            }
            (None, Ok(_)) => panic!("{file_name} exists, but shouldn't"),
            (None, Err(err)) if err.kind() == ErrorKind::NotFound => {}
            (_, Err(err)) => panic!("{err:?}"),
        }
    }
}

#[proptest]
fn fatfs_check(input: TestInput) {
    let cidata = input.as_cidata();
    let volume = cidata.generate().unwrap();
    let fs =
        fatfs::FileSystem::new(Cursor::new(volume), FsOptions::new()).unwrap();
    check_file_system(&cidata, &fs);

    assert_eq!(fs.fat_type(), FatType::Fat12);
    let status_flags = fs.read_status_flags().unwrap();
    assert!(!status_flags.dirty());
    assert!(!status_flags.io_error());
    assert_eq!(fs.volume_label(), "cidata");
    assert_eq!(
        fs.read_volume_label_from_root_dir().unwrap().unwrap(),
        "cidata"
    );
}

#[proptest]
#[ignore = "OS integration test, escalated privileges likely required"]
fn os_check(input: TestInput) {
    struct DropCmd<'a>(Cmd<'a>);

    impl Drop for DropCmd<'_> {
        fn drop(&mut self) {
            if let Err(err) = self.0.run() {
                if std::thread::panicking() {
                    eprintln!("{err}");
                } else {
                    panic!("{err}");
                }
            }
        }
    }

    let cidata = input.as_cidata();
    let mut volume = cidata.generate().unwrap();
    if cfg!(target_os = "windows") {
        append_vhd_footer(&mut volume);
    }

    let tempdir = tempfile::tempdir().unwrap();
    let src = tempdir.path().join("testfat.img");
    std::fs::write(&src, &volume).unwrap();

    let sh = Shell::new().unwrap();
    let (mountpoint, unmount) = if cfg!(target_os = "windows") {
        let drive_letter = cmd!(
            sh,
            "powershell -Command '$ErrorActionPreference = \"Stop\"; (\
                Mount-DiskImage \
                    -ImagePath \"'{src}'\" \
                    -StorageType VHD \
                    -Access ReadOnly \
                    -PassThru \
                | Get-Disk | Get-Partition\
            ).DriveLetter'"
        )
        .read()
        .unwrap();
        let mountpoint = PathBuf::from(format!("{drive_letter}:\\"));
        let unmount =
            cmd!(sh, "powershell -Command Dismount-DiskImage -ImagePath {src}");
        (mountpoint, unmount)
    } else {
        let mountpoint = tempdir.path().join("mountpoint");
        std::fs::create_dir(&mountpoint).unwrap();

        let (mount, unmount) = if cfg!(target_os = "illumos") {
            (
                cmd!(sh, "pfexec mount -F pcfs -o ro {src} {mountpoint}"),
                cmd!(sh, "pfexec umount {mountpoint}"),
            )
        } else if cfg!(target_os = "linux") {
            let blkid = cmd!(sh, "blkid {src}").read().unwrap();
            assert!(blkid.contains("TYPE=\"vfat\""));
            cmd!(sh, "fsck.fat -n {src}").run().unwrap();
            (
                cmd!(sh, "sudo mount -o ro {src} {mountpoint}"),
                cmd!(sh, "sudo umount {mountpoint}"),
            )
        } else if cfg!(target_os = "macos") {
            (
                cmd!(
                    sh,
                    "diskutil image attach -readOnly -mountPoint {mountpoint} {src}"
                ),
                cmd!(sh, "diskutil eject {mountpoint}"),
            )
        } else {
            unimplemented!();
        };
        mount.run().unwrap();
        (mountpoint, unmount)
    };

    let _unmount = DropCmd(unmount);
    check_file_system(&cidata, &MountedFileSystem(mountpoint));
}

// In order to mount a FAT image on Windows, we need to convert it to a VHD. The
// easiest way to do this is to simply append a footer.
// https://go.microsoft.com/fwlink/p/?linkid=137171
fn append_vhd_footer(volume: &mut Vec<u8>) {
    // spec: https://go.microsoft.com/fwlink/p/?linkid=137171

    #[derive(Clone, Copy, bytemuck::AnyBitPattern, bytemuck::NoUninit)]
    #[repr(C)]
    struct VhdFooter {
        cookie: [u8; 8],
        features: [u8; 4],
        file_format_version: [u8; 4],
        data_offset: [u8; 8],
        time_stamp: [u8; 4],
        creator_application: [u8; 4],
        creator_version: [u8; 4],
        creator_host_os: [u8; 4],
        original_size: [u8; 8],
        current_size: [u8; 8],
        cylinders: [u8; 2],
        heads: u8,
        sectors_per_track: u8,
        disk_type: [u8; 4],
        checksum: [u8; 4],
        unique_id: [u8; 16],
        saved_state: u8,
        reserved: [u8; 427],
    }

    let total_sectors = volume.len() / 512;
    // per the spec, sectors_per_track is always 17, 31, 63, or 255. given the
    // file size limit of a FAT12 file system, the given calculation always
    // results in 17 sectors per track.
    let sectors_per_track = 17u8;
    let cylinders_times_heads = total_sectors / usize::from(sectors_per_track);
    let heads =
        u8::try_from(cylinders_times_heads.div_ceil(1024)).unwrap().max(4);
    assert!(heads <= 16);
    let cylinders =
        u16::try_from(cylinders_times_heads.div_ceil(usize::from(heads)))
            .unwrap()
            .max(1);
    // pad the image so that the disk image is not truncated
    let len = usize::from(cylinders)
        * usize::from(heads)
        * usize::from(sectors_per_track)
        * 512;
    assert!(len >= volume.len());
    volume.resize(len, 0);
    let size = u64::try_from(len).unwrap();

    let mut footer = VhdFooter {
        cookie: *b"conectix",
        features: 0x0000_0002_u32.to_be_bytes(),
        file_format_version: 0x0001_0000_u32.to_be_bytes(),
        data_offset: u64::MAX.to_be_bytes(), // indicates fixed-size disk
        time_stamp: 0u32.to_be_bytes(),
        creator_application: *b"meow",
        creator_version: 0x0001_0000u32.to_be_bytes(),
        creator_host_os: *b"Wi2k",
        original_size: size.to_be_bytes(),
        current_size: size.to_be_bytes(),
        cylinders: cylinders.to_be_bytes(),
        heads,
        sectors_per_track,
        disk_type: 2u32.to_be_bytes(), // "Fixed hard disk"
        checksum: 0u32.to_be_bytes(),
        unique_id: Uuid::new_v4().into_bytes(),
        saved_state: 0,
        reserved: [0; 427],
    };
    let bytes = bytemuck::bytes_of_mut(&mut footer);
    footer.checksum = bytes
        .iter()
        .copied()
        .fold(0u32, |acc, b| acc.wrapping_add(b.into()))
        .not()
        .to_be_bytes();
    volume.extend_from_slice(bytemuck::bytes_of(&footer));
}

#[test]
fn generate_test_archive() {
    macro_rules! header {
        ($size:expr) => {{
            let mut header = Header::new_gnu();
            header.set_size($size.try_into().unwrap());
            header.set_mode(0o644);
            header
        }};
    }

    static COUNT: AtomicUsize = AtomicUsize::new(1);

    let config = Config {
        test_name: Some("integration::generate_test_archive"),
        source_file: Some(file!()),
        ..proptest::test_runner::contextualize_config(Config::default())
    };
    let digits = config.cases.checked_ilog10().unwrap_or(0) as usize + 1;
    let mut runner = TestRunner::new(config);

    let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cidata")
        .join("tests.tar.gz");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    AtomicFile::new(&path, OverwriteBehavior::AllowOverwrite)
        .write(|file| {
            let archive = Mutex::new(tar::Builder::new(GzEncoder::new(
                file,
                Compression::default(),
            )));
            runner
                .run(&proptest::arbitrary::any::<TestInput>(), |input| {
                    let id = COUNT.fetch_add(1, Ordering::Relaxed);
                    let dir = PathBuf::from(format!("{id:0digits$}"));
                    let cidata = input.as_cidata();
                    let volume = cidata.generate().unwrap();
                    let mut archive = archive.lock().unwrap();
                    archive
                        .append_data(
                            &mut header!(volume.len()),
                            dir.join("cidata.img"),
                            volume.as_slice(),
                        )
                        .unwrap();
                    for (file_name, data) in [
                        ("meta-data", cidata.meta_data),
                        ("network-config", cidata.network_config),
                        ("user-data", cidata.user_data),
                        ("vendor-data", cidata.vendor_data),
                    ] {
                        let Some(data) = data else { continue };
                        archive
                            .append_data(
                                &mut header!(data.len()),
                                dir.join("expected").join(file_name),
                                data,
                            )
                            .unwrap();
                    }
                    Ok(())
                })
                .unwrap();
            std::io::Result::Ok(())
        })
        .unwrap();
}
