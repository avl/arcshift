use arc_swap::{ArcSwap, Cache};
use arcshift::ArcShift;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::ops::Deref;
use std::sync::{Arc, Mutex, RwLock};

fn std_arc_bench(c: &mut Criterion) {
    let ac = Arc::new(42u32);
    c.bench_function("std_arc_bench", |b| {
        b.iter(|| {
            _ = black_box(*ac);
        })
    });
}

fn mutex_bench(c: &mut Criterion) {
    let ac = Mutex::new(42u32);
    c.bench_function("mutex", |b| b.iter(|| ac.lock().unwrap()));
}
fn rwlock_read_bench(c: &mut Criterion) {
    let ac = RwLock::new(42u32);
    c.bench_function("rwlock_read", |b| {
        b.iter(|| {
            let guard = ac.read().unwrap();
            *guard
        })
    });
}
fn rwlock_contended_read_bench(c: &mut Criterion) {
    let ac = Arc::new(RwLock::new(42u32));
    let ac_clone = ac.clone();
    std::thread::spawn(move || loop {
        drop(black_box(ac_clone.read()));
    });
    c.bench_function("rwlock_contended_read", |b| {
        b.iter(|| {
            let guard = ac.read().unwrap();
            *guard
        })
    });
}
fn rwlock_write_bench(c: &mut Criterion) {
    let ac = RwLock::new(42u32);
    c.bench_function("rwlock_write", |b| {
        b.iter(|| {
            let mut guard = ac.write().unwrap();
            *guard = 43;
        })
    });
}
fn arcshift_bench(c: &mut Criterion) {
    let mut ac = ArcShift::new(42u32);
    c.bench_function("arcshift_get", |b| b.iter(|| *ac.get()));
}
fn arcshift_contended_bench(c: &mut Criterion) {
    let mut ac = ArcShift::new(42u32);
    let mut ac_clone = ac.clone();
    std::thread::spawn(move || loop {
        black_box(ac_clone.get());
    });
    c.bench_function("arcshift_contended_get", |b| b.iter(|| *ac.get()));
}
fn arcshift_update_bench(c: &mut Criterion) {
    let mut ac = ArcShift::new(42u32);
    c.bench_function("arcshift_update", |b| {
        b.iter(|| {
            ac.update(43);
        })
    });
}
fn arcshift_shared_bench(c: &mut Criterion) {
    let ac = ArcShift::new(42u32);
    c.bench_function("arcshift_shared_get", |b| b.iter(|| *ac.shared_get()));
}

fn arcshift_shared_stale_bench(c: &mut Criterion) {
    let mut ac_base = ArcShift::new(42u32);
    let ac = ac_base.clone();
    ac_base.update(43);
    c.bench_function("arcshift_shared_stale_get", |b| b.iter(|| *ac.shared_get()));
}

fn arcshift_shared_non_reloading_bench(c: &mut Criterion) {
    let ac = ArcShift::new(42u32);
    c.bench_function("arcshift_shared_non_reloading_get", |b| {
        b.iter(|| *ac.shared_non_reloading_get())
    });
}

fn arcswap_bench(c: &mut Criterion) {
    let ac = Arc::new(ArcSwap::from_pointee(42));
    c.bench_function("arc_swap", |b| {
        b.iter(|| {
            let loaded = ac.load();
            let arc = loaded.deref();
            let x: i32 = *(*arc).deref();
            x
        })
    });
}
fn arcswap_stale_bench(c: &mut Criterion) {
    let shared = Arc::new(ArcSwap::from_pointee(42));
    let shared2 = shared.clone();

    let jh = std::thread::spawn(move || {
        shared2.store(Arc::new(43));
    });
    jh.join().unwrap();

    c.bench_function("arc_swap_stale", |b| {
        b.iter(|| {
            let arc = shared.load();
            let x: i32 = *(*arc).deref();
            black_box(x)
        })
    });
}

fn arcswap_cached_bench(c: &mut Criterion) {
    let shared = Arc::new(ArcSwap::from_pointee(42));
    let mut cache = Cache::new(Arc::clone(&shared));

    c.bench_function("arc_swap(w cache)", |b| {
        b.iter(|| {
            let arc = cache.load();
            let x: i32 = *(*arc).deref();
            x
        })
    });
}

fn arcswap_update(c: &mut Criterion) {
    let ac = ArcSwap::from_pointee(42u32);
    c.bench_function("arc_swap_update", |b| b.iter(|| ac.swap(Arc::new(43u32))));
}
criterion_group!(
    benches,
    arcshift_shared_bench,
    arcshift_shared_stale_bench,
    arcshift_shared_non_reloading_bench,
    std_arc_bench,
    rwlock_write_bench,
    arcshift_bench,
    arcshift_contended_bench,
    arcshift_update_bench,
    arcswap_bench,
    arcswap_stale_bench,
    mutex_bench,
    rwlock_read_bench,
    rwlock_contended_read_bench,
    arcswap_update,
);
criterion_main!(benches);
