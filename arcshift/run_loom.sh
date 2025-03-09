LOOM_MAX_BRANCHES=100000 LOOM_MAX_PREEMPTIONS=1 RUSTFLAGS="--cfg loom" cargo nextest run --features=loom,validate --release $@
