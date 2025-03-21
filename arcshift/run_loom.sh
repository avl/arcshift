LOOM_MAX_BRANCHES=4000 LOOM_MAX_PREEMPTIONS=2 RUSTFLAGS="--cfg loom" cargo nextest run --features=loom,validate --release $@
