set -euo pipefail
mkdir -p miri
export MIRI_PAR_ARG=$@
seq 10000|xargs -IXXX -n1 -P32 bash -c "cargo miri test $MIRI_PAR_ARG --many-seeds=XXX00..XXX99 >miri/miriXXX.out 2>&1  || exit 255"

