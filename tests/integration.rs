// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::fmt::Debug;
use std::io::Cursor;
use std::io::ErrorKind;
use std::io::Read;
use std::ops::Deref;
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
    let volume = cidata.generate().unwrap();

    let tempdir = tempfile::tempdir().unwrap();
    let src = tempdir.path().join("testfat.img");
    std::fs::write(&src, &volume).unwrap();
    let mountpoint = tempdir.path().join("mountpoint");
    std::fs::create_dir(&mountpoint).unwrap();

    let sh = Shell::new().unwrap();
    let (mount, unmount) = if cfg!(target_os = "illumos") {
        (
            cmd!(sh, "pfexec mount -F pcfs {src} {mountpoint}"),
            cmd!(sh, "pfexec umount {mountpoint}"),
        )
    } else if cfg!(target_os = "linux") {
        let blkid = cmd!(sh, "blkid {src}").read().unwrap();
        assert!(blkid.contains("TYPE=\"vfat\""));
        cmd!(sh, "fsck.fat -n {src}").run().unwrap();
        (
            cmd!(sh, "sudo mount {src} {mountpoint}"),
            cmd!(sh, "sudo umount {mountpoint}"),
        )
    } else if cfg!(target_os = "macos") {
        (
            cmd!(sh, "diskutil image attach -mountPoint {mountpoint} {src}"),
            cmd!(sh, "diskutil eject {mountpoint}"),
        )
    } else {
        unimplemented!();
    };
    mount.run().unwrap();
    let _unmount = DropCmd(unmount);
    check_file_system(&cidata, &MountedFileSystem(mountpoint));
}

#[test]
fn generate_test_archive() {
    macro_rules! header {
        ($size:expr) => {{
            let mut header = Header::new_gnu();
            header.set_size($size.try_into().unwrap());
            header
        }};
    }

    static COUNT: AtomicUsize = AtomicUsize::new(1);

    let config = Config {
        test_name: Some("integration::generate_test_zip"),
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
