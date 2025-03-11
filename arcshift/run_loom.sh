LOOM_MAX_BRANCHES=10000 LOOM_MAX_PREEMPTIONS=3 RUSTFLAGS="--cfg loom" cargo nextest run --features=loom,validate --release $@
