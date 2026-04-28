#!/bin/bash
#:
#: name = "helios"
#: variety = "basic"
#: target = "helios-2.0"
#: rust_toolchain = true
#: output_rules = [
#:	"=/work/oxidecomputer/cidata/target/tmp/cidata/tests.tar.gz",
#: ]
#:
#: [[publish]]
#: series = "generated-tests"
#: name = "tests.tar.gz"
#: from_output = "/work/oxidecomputer/cidata/target/tmp/cidata/tests.tar.gz"
#:

set -o errexit
set -o pipefail
set -o xtrace

cargo --version
rustc --version
curl -sSfL --retry 10 https://get.nexte.st/0.9/illumos | gunzip | tar -xvf - -C ~/.cargo/bin

export CARGO_TERM_COLOR="always"
export RUSTFLAGS="-D warnings"
cargo nextest run --run-ignored all
