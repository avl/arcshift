LOOM_MAX_BRANCHES=100000 LOOM_MAX_PREEMPTIONS=2 RUSTFLAGS="--cfg loom" cargo nextest run --features=loom,validate --release $@
