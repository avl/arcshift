LOOM_MAX_PREEMPTIONS=3 RUSTFLAGS="--cfg loom" cargo test --release  -- --nocapture $@
