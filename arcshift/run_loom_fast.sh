LOOM_MAX_PREEMPTIONS=1 RUSTFLAGS="--cfg loom" cargo test --lib --tests --features=disable_slow_tests --release  -- --nocapture $@
