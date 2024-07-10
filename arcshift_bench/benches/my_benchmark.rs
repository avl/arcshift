use std::ops::Deref;
use std::sync::Arc;
use arc_swap::{ArcSwap, Cache};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use arcshift::ArcShift;

fn arcshift_bench(c: &mut Criterion) {
    let mut ac = ArcShift::new(42u32);
    c.bench_function("arcshift", |b| b.iter(||{
        let value = ac.get();
        _ = black_box(value);
        }
    ));
}
fn arcswap_bench(c: &mut Criterion) {
    let ac = ArcSwap::from_pointee(42u32);
    c.bench_function("arc_swap", |b| b.iter(||{
        let loaded = ac.load();
        let arc = loaded.deref();
        let val = *(*arc).deref();
        _ = black_box(val);
    }
    ));
}
fn arcswap_cached_bench(c: &mut Criterion) {
    let shared = Arc::new(ArcSwap::from_pointee(42));
    let mut cache = Cache::new(Arc::clone(&shared));

    c.bench_function("arc_swap(w cache)", |b| b.iter(||{
        let arc = cache.load();
        let val = *(*arc).deref();
        _ = black_box(val);
    }
    ));
}

criterion_group!(benches, arcshift_bench, arcswap_bench, arcswap_cached_bench);
criterion_main!(benches);
