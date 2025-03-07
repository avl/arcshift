LOOM_MAX_PREEMPTIONS=3 RUSTFLAGS="--cfg loom" cargo nextest run --features=loom,disable_slow_tests,validate --release $@
