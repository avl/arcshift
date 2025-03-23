#!/usr/bin/bash
set -euo pipefail
mkdir -p miri
export MIRI_PAR_ARG=$@
seq 10000|xargs -IXXX -n1 -P16 bash -c "MIRIFLAGS=-Zmiri-many-seeds=XXX00..XXX99 cargo miri test $MIRI_PAR_ARG >miri/miriXXX.out 2>&1  || exit 255"

