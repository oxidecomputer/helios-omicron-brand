#!/bin/bash
#:
#: name = "pkg"
#: variety = "basic"
#: target = "helios-2.0-20240204"
#: rust_toolchain = "stable"
#: output_rules = [
#:	"=/out/omicron-brand.p5p",
#:	"=/out/omicron-brand.p5p.sha256.txt",
#: ]
#:
#: [[publish]]
#: from_output = "/out/omicron-brand.p5p"
#: series = "pkg"
#: name = "omicron-brand.p5p"
#:
#: [[publish]]
#: from_output = "/out/omicron-brand.p5p.sha256.txt"
#: series = "pkg"
#: name = "omicron-brand.p5p.sha256.txt"
#:

set -o errexit
set -o pipefail
set -o xtrace

cargo --version
rustc --version

pfexec mkdir -p /out
pfexec chown "$LOGNAME" /out

banner build
gmake package
gmake archive

banner output
cp packages/omicron-brand-*.p5p /out/omicron-brand.p5p
digest -a sha256 /out/omicron-brand.p5p > /out/omicron-brand.p5p.sha256.txt
