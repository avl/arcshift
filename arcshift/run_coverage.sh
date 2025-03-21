RUSTFLAGS="--cfg coverage" cargo tarpaulin -p arcshift  --lib --tests --features --features=shuttle,validate,disable_slow_tests,debug -o Html
