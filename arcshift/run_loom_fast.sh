LOOM_MAX_PREEMPTIONS=1 RUSTFLAGS="--cfg loom" cargo test --release  -- --nocapture
