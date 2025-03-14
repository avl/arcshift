LOOM_MAX_BRANCHES=2000 LOOM_MAX_PREEMPTIONS=1 RUSTFLAGS="--cfg loom" cargo nextest run --lib --tests --features=loom,disable_slow_tests,validate --release $@
