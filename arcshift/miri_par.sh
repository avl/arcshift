set -euo pipefail
mkdir -p miri
seq 10000|xargs -IXXX -n1 -P32 bash -c "cargo miri test --many-seeds=XXX00..XXX99 >miri/miriXXX.out 2>&1  || exit 255"

