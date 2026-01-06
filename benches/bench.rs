// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![expect(clippy::needless_pass_by_value)]

use std::hint::black_box;

use cidata::Cidata;
use cidata::Error;
use gungraun::library_benchmark;
use gungraun::library_benchmark_group;
use gungraun::main;

const EMPTY: Cidata<'_> = Cidata::new();
const NORMAL: Cidata<'_> = Cidata {
    meta_data: Some(&[0; 1000]),
    user_data: Some(&[0; 4000]),
    ..Cidata::new()
};
const FULL: Cidata<'_> = Cidata {
    meta_data: Some(&[0; 1000]),
    network_config: Some(&[0; 2000]),
    user_data: Some(&[0; 4000]),
    vendor_data: Some(&[0; 3000]),
};

#[library_benchmark]
#[bench::empty(EMPTY)]
#[bench::normal(NORMAL)]
#[bench::full(FULL)]
fn bench_generate(cidata: Cidata<'_>) -> Result<Vec<u8>, Error> {
    black_box(cidata.generate())
}

library_benchmark_group!(
    name = bench_cidata_group;
    benchmarks = bench_generate
);

main!(library_benchmark_groups = bench_cidata_group);
