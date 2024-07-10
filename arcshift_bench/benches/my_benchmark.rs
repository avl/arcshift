use std::ops::Deref;
use std::sync::{Arc, Mutex, RwLock};
use arc_swap::{ArcSwap, Cache};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use arcshift::ArcShift;

fn mutex_bench(c: &mut Criterion) {
    let ac = Mutex::new(42u32);
    c.bench_function("mutex", |b| b.iter(||{
        let guard = ac.lock().unwrap();
        let value: u32 = *guard;
        _ = black_box(value);
    }
    ));
}

fn rwlock_bench(c: &mut Criterion) {
    let ac = RwLock::new(42u32);
    c.bench_function("rwlock", |b| b.iter(||{
        let guard = ac.read().unwrap();
        let value: u32 = *guard;
        _ = black_box(value);
    }
    ));
}

fn arcshift_bench(c: &mut Criterion) {
    let mut ac = ArcShift::new(42u32);
    c.bench_function("arcshift", |b| b.iter(||{
        let value = ac.get();
        _ = black_box(value);
        }
    ));
}
fn arcshift_shared_bench(c: &mut Criterion) {
    let ac = ArcShift::new(42u32);
    c.bench_function("arcshift_shared", |b| b.iter(||{
        let value = ac.shared_get();
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

criterion_group!(benches, arcshift_bench, arcswap_bench, arcswap_cached_bench, mutex_bench, rwlock_bench, arcshift_shared_bench);
criterion_main!(benches);
