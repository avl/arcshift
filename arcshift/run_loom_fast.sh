LOOM_MAX_PREEMPTIONS=1 RUSTFLAGS="--cfg loom" cargo test --lib --tests --features=loom,disable_slow_tests,validate --release  -- --nocapture $@
