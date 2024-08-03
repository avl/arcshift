LOOM_MAX_PREEMPTIONS=3 RUSTFLAGS="--cfg loom" cargo test --lib --tests --features=disable_slow_tests,validate --release  -- --nocapture $@
