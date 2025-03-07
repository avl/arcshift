seq 10000|xargs -IXXX -n1 -P32 bash -c "cargo miri test --many-seeds=XXX00..XXX99|tee miriXXX.out || exit 255"

