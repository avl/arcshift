#![deny(warnings)]
#![allow(dead_code)]
#![allow(unused_imports)]
use super::*;
use crossbeam_channel::bounded;
use leak_detection::{InstanceSpy, InstanceSpy2, SpyOwner2};
use rand::prelude::StdRng;
use rand::{Rng, SeedableRng};
use std::alloc::Layout;
use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::hint::black_box;
use std::mem::MaybeUninit;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

mod custom_fuzz;
pub(crate) mod leak_detection;
mod race_detector;

// All tests are wrapped by these 'model' calls.
// This is needed to make the tests runnable from within the Shuttle and Loom frameworks.

#[cfg(all(not(loom), not(feature = "shuttle")))]
fn model(x: impl FnOnce()) {
    x()
}
#[cfg(all(not(loom), not(feature = "shuttle")))]
fn model2(x: impl FnOnce(), _repro: Option<&str>) {
    x()
}
#[cfg(loom)]
fn model(x: impl Fn() + 'static + Send + Sync) {
    loom::model(x)
}
#[cfg(loom)]
fn model2(x: impl Fn() + 'static + Send + Sync, _repro: Option<&str>) {
    loom::model(x)
}

#[cfg(all(feature = "shuttle", coverage))]
const SHUTTLE_ITERATIONS: usize = 50;
#[cfg(all(feature = "shuttle", not(coverage)))]
const SHUTTLE_ITERATIONS: usize = 2;

#[cfg(feature = "shuttle")]
fn model(x: impl Fn() + 'static + Send + Sync) {
    shuttle::check_pct(x, SHUTTLE_ITERATIONS, 4);
}
#[cfg(feature = "shuttle")]
fn model2(x: impl Fn() + 'static + Send + Sync, repro: Option<&str>) {
    if let Some(repro) = repro {
        shuttle::replay(x, repro);
    } else {
        shuttle::check_pct(x, SHUTTLE_ITERATIONS, 4);
    }
}

// Here follows some simple basic tests

#[cfg(not(any(loom, feature="shuttle")))]
mod simple
{
#[test]
fn simple_get() {
    let mut shift = ArcShift::new(42u32);
    assert_eq!(*shift.get(), 42u32);
}
#[test]
fn simple_box() {
        let mut shift = ArcShift::from_box(Box::new(42u32));
        assert_eq!(*shift.get(), 42u32);
}
#[test]
fn simple_unsized() {
        let biggish = vec![1u32, 2u32].into_boxed_slice();
        let mut shift = ArcShift::from_box(biggish);
        debug_println!("Drop");
        assert_eq!(shift.get(), &vec![1, 2]);
}

trait ExampleTrait {
    fn call(&self) -> u32;
}
struct ExampleStruct {
    x: u32,
}
impl ExampleTrait for ExampleStruct {
    fn call(&self) -> u32 {
        self.x
    }
}
#[test]
fn simple_unsized_closure() {
        let boxed_trait: Box<dyn ExampleTrait> = Box::new(ExampleStruct { x: 42 });
        let mut shift = ArcShift::from_box(boxed_trait);
        debug_println!("Drop");
        assert_eq!(shift.get().call(), 42);
}
#[test]
fn simple_unsized_str() {
        let boxed_str: Box<str> = Box::new("hello".to_string()).into_boxed_str();
        let mut shift = ArcShift::from_box(boxed_str);
        debug_println!("Drop");
        assert_eq!(shift.get(), "hello");
}
use crate::cell::ArcShiftCell;
use std::cell::{Cell, RefCell};

thread_local! {

    static THREADLOCAL_FOO: ArcShiftCell<String> = ArcShiftCell::new(String::new());
}

#[cfg(not(any(loom, feature = "shuttle")))]
//This test doesn't work in shuttle or loom, since the lazy drop of the threadlocal ends up happening outside of the shuttle model
#[test]
fn simple_threadlocal_cell() {
        let shift = ArcShift::new("hello".to_string());
        THREADLOCAL_FOO.with(|local| {
            local.assign(&shift).unwrap();
        });
        THREADLOCAL_FOO.with(|local| {
            local.get(|value| {
                assert_eq!(value, "hello");
            });
        });
        debug_println!("Drop");
}

#[test]
fn simple_cell() {
        let owner = SpyOwner2::new();
        {
            let mut root = ArcShift::new(owner.create("root"));
            let cell = ArcShiftCell::from_arcshift(root.clone());
            cell.get(|val| {
                assert_eq!(val.str(), "root");
            });
            root.update(owner.create("new"));

            assert_eq!(owner.count(), 2);

            cell.reload();
            assert_eq!(owner.count(), 1);

            cell.get(|val| {
                assert_eq!(val.str(), "new");
            });

            root.update(owner.create("new2"));
            assert_eq!(owner.count(), 2);

            cell.get(|val| {
                assert_eq!(val.str(), "new2");
            });

            assert_eq!(owner.count(), 1);
        }
        owner.validate();
}
#[test]
fn simple_cell_handle() {
        let owner = SpyOwner2::new();
        {
            let mut root = ArcShift::new(owner.create("root"));
            let cell = ArcShiftCell::from_arcshift(root.clone());
            let r = cell.borrow();
            assert_eq!(r.str(), "root");
            assert_eq!(owner.count(), 1);
            root.update(owner.create("new"));
            assert_eq!(owner.count(), 2); // Since we haven't dropped 'r', its value must be kept
            drop(r);
            assert_eq!(owner.count(), 1); //'cell' should now have been reloaded.
        }
}

#[test]
fn simple_multiple_cell_handles() {
        let owner = SpyOwner2::new();
        {
            let mut root = ArcShift::new(owner.create("root"));
            let cell = ArcShiftCell::from_arcshift(root.clone());
            {
                let r = cell.borrow();
                assert_eq!(r.str(), "root");
                assert_eq!(owner.count(), 1);
            }

            root.update(owner.create("A"));
            assert_eq!(owner.count(), 2);

            {
                let r = cell.borrow();
                assert_eq!(r.str(), "A");
                assert_eq!(owner.count(), 1);
            }

            {
                let r = cell.borrow();
                root.update(owner.create("B"));
                assert_eq!(r.str(), "B");
                assert_eq!(owner.count(), 1);
            }

            {
                let r1 = cell.borrow();
                let r2 = cell.borrow();
                root.update(owner.create("C"));
                assert_eq!(root.str(), "C");
                assert_eq!(r1.str(), "B");
                assert_eq!(r2.str(), "B");
                assert_eq!(owner.count(), 2); //Because we have two references, we can't reload.
            }
            {
                let r1 = cell.borrow();
                let r2 = cell.borrow();
                assert_eq!(r1.str(), "C");
                assert_eq!(r2.str(), "C");
            }
            assert_eq!(owner.count(), 1); //But when the last ref is dropped, we do reload
        }
}

#[test]
fn simple_cell_recursion() {
        let owner = SpyOwner2::new();
        {
            let mut root = ArcShift::new(owner.create("root"));
            let cell = ArcShiftCell::from_arcshift(root.clone());
            cell.get(|val| {
                assert_eq!(val.str(), "root");
                assert!(cell.assign(&ArcShift::new(owner.create("dummy"))).is_err());
                cell.get(|val| {
                    assert_eq!(val.str(), "root");
                    root.update(owner.create("B"));
                    cell.get(|val| {
                        assert_eq!(val.str(), "root");
                    });
                    assert_eq!(val.str(), "root");
                });
                assert_eq!(val.str(), "root");
            });
            cell.get(|val| {
                assert_eq!(val.str(), "B");
            });
        }
        owner.validate();
}
#[test]
fn simple_cell_assign() {
        let owner = SpyOwner2::new();
        {
            let cell = ArcShiftCell::new(owner.create("original"));
            let new_value = ArcShift::new(owner.create("new"));

            cell.get(|val| {
                assert_eq!(val.str(), "original");
                assert!(cell.assign(&ArcShift::new(owner.create("dummy"))).is_err());
            });

            cell.assign(&new_value).unwrap();

            cell.get(|val| assert_eq!(val.str(), "new"));
        }
}

#[test]
fn simple_rcu() {
        let mut shift = ArcShift::new(42u32);
        shift.rcu(|x| x + 1);
        assert_eq!(*shift.get(), 43u32);
}

#[test]
fn simple_rcu_maybe() {
        let mut shift = ArcShift::new(42u32);
        assert!(shift.rcu_maybe(|x| Some(x + 1)));
        assert_eq!(*shift.get(), 43u32);
        assert_eq!(shift.rcu_maybe(|_x| None), false);
        assert_eq!(*shift.get(), 43u32);
}
#[test]
fn simple_rcu_maybe2() {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(shift.rcu_maybe(|x| Some(x + 1)), true);
        assert_eq!(shift.rcu_maybe(|_x| None), false);
        assert_eq!(*shift.get(), 43u32);
        assert_eq!(shift.rcu_maybe(|_x| None), false);
        assert_eq!(*shift.get(), 43u32);
}

#[test]
fn simple_rcu_maybe3() {
        let mut shift = ArcShift::new(42u32);
        let mut shift2 = shift.clone();
        assert_eq!(shift.rcu_maybe(|x| Some(x + 1)), true);
        assert_eq!(shift.rcu_maybe(|_x| None), false);
        assert_eq!(*shift.get(), 43u32);
        assert_eq!(shift.rcu_maybe(|_x| None), false);
        assert_eq!(*shift.get(), 43u32);
        assert_eq!(*shift2.get(), 43u32);
}
#[test]
fn simple_deref() {
        let shift = ArcShift::new(42u32);
        assert_eq!(*shift, 42u32);
}
#[test]
fn simple_get4() {
        let shift = ArcShift::new(42u32);
        assert_eq!(*shift.shared_get(), 42u32);
}
#[test]
fn simple_get_mut() {
        let mut shift = ArcShift::new(42u32);
        // Uniquely owned values can be modified using 'try_get_mut'.
        assert_eq!(*shift.try_get_mut().unwrap(), 42);
}
#[test]
fn simple_zerosized() {
        let mut shift = ArcShift::new(());
        assert_eq!(*shift.get(), ());
        shift.update(());
        assert_eq!(*shift.get(), ());
}
#[test]
fn simple_update() {
        let mut shift = ArcShift::new(42);
        let old = &*shift;
        shift.update(*old + 4);
        shift.reload();
}

#[test]
fn simple_get_mut2() {
        let mut shift = ArcShift::new(42u32);
        let mut shift2 = shift.clone();
        shift2.update(43);
        assert_eq!(shift.try_get_mut(), None);
}
#[test]
fn simple_get_mut3() {
        let mut shift = ArcShift::new(42u32);
        let _shift2 = ArcShift::downgrade(&shift);
        shift.update(43);
        assert_eq!(shift.try_get_mut(), None);
}
#[test]
fn simple_get_mut4_mut() {
        let mut shift = ArcShift::new(42u32);
        let mut shift2 = shift.clone();
        shift2.update(43);
        drop(shift2);
        assert_eq!(shift.try_get_mut(), Some(&mut 43));
}
#[test]
fn simple_try_into() {
        let shift = ArcShift::new(42u32);
        // Uniquely owned values can be moved out without being dropped
        assert_eq!(shift.try_into_inner().unwrap(), 42);
}

#[test]
fn simple_into_inner() {
        let shift = ArcShift::new(42u32);
        // Uniquely owned values can be moved out without being dropped
        assert_eq!(shift.try_into_inner().unwrap(), 42);
}
#[test]
fn simple_into_inner2() {
        let shift = ArcShift::new(42u32);
        let mut shift2 = shift.clone();
        shift2.update(43);
        drop(shift2);
        assert_eq!(shift.try_into_inner(), Some(43));
}
#[test]
fn simple_failing_into_inner() {
        let shift = ArcShift::new(42u32);
        let _shift2 = shift.clone();

        assert_eq!(shift.try_into_inner(), None);
}
#[test]
fn simple_failing_into_inner2() {
        let shift = ArcShift::new(42u32);
        let mut shift2 = shift.clone();
        shift2.update(43);

        assert_eq!(shift.try_into_inner(), None);
}

#[test]
// There's no point in running this test under shuttle/loom,
// and since it can take some time, let's just disable it.
#[cfg(not(any(loom, feature = "shuttle")))]
fn simple_large() {
        #[cfg(not(miri))]
        const SIZE: usize = 10_000_000;
        #[cfg(miri)]
        const SIZE: usize = 10_000;

        let layout = Layout::new::<MaybeUninit<[u64; SIZE]>>();

        let ptr: *mut [u64; SIZE] = unsafe { std::alloc::alloc(layout) } as *mut [u64; SIZE];
        for i in 0..SIZE {
            unsafe { *(&mut (*ptr)).get_unchecked_mut(i) = 42 };
        }
        let bigbox = unsafe { Box::from_raw(ptr) };
        let mut shift = ArcShift::from_box(bigbox);
        let value = shift.get();
        assert_eq!(value[0], 42);
}
#[test]
fn simple_get2() {
        let mut shift = ArcShift::from_box(Box::new(42u32));
        assert_eq!(*shift.get(), 42u32);
}
#[test]
fn simple_get3() {
        let mut shift = ArcShift::new("hello".to_string());
        assert_eq!(shift.get(), "hello");
}
#[test]
fn simple_update0() {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
        shift.update(43);
        assert_eq!(*shift.get(), 43u32);
}
#[test]
fn simple_update_boxed() {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
        shift.update_box(Box::new(43));
        assert_eq!(*shift.get(), 43u32);
}

#[test]
fn simple_update2() {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
        shift.update(43);
        shift.update(44);
        shift.update(45);
        assert_eq!(*shift.get(), 45u32);
}
#[test]
fn simple_update3() {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
        shift.update_box(Box::new(45));
        assert_eq!(*shift.get(), 45u32);
}

#[test]
fn simple_update5() {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
        shift.update(45);
        assert_eq!(*shift.get(), 45u32);
}

#[test]
fn simple_upgrade3a1() {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let shift1 = ArcShift::new(InstanceSpy::new(count.clone()));
        let shiftlight = ArcShift::downgrade(&shift1);

        debug_println!("==== running shift.get() = ");
        let mut shift2 = shiftlight.upgrade().unwrap();
        debug_println!("==== running arc.update() = ");
        shift2.update(InstanceSpy::new(count.clone()));

        unsafe { ArcShift::debug_validate(&[&shift1, &shift2], &[&shiftlight]) };
        debug_println!("==== Instance count: {}", count.load(Ordering::SeqCst));
        drop(shift1);
        assert_eq!(count.load(Ordering::SeqCst), 1); // The 'ArcShiftLight' should *not* keep any version alive
        debug_println!("==== drop arc =");
        drop(shift2);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        debug_println!("==== drop shiftroot =");
        drop(shiftlight);
        assert_eq!(count.load(Ordering::SeqCst), 0);
}
#[test]
fn simple_upgrade3a0() {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut arc = ArcShift::new(InstanceSpy::new(count.clone()));
        for _ in 0..10 {
            arc.update(InstanceSpy::new(count.clone()));
            debug_println!("Instance count: {}", count.load(Ordering::SeqCst));
            arc.reload();
            assert_eq!(count.load(Ordering::Relaxed), 1); // The 'arc' should *not* keep any version alive
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        drop(arc);
        assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn simple_upgrade4b() {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let mut shift = ArcShift::new(InstanceSpy::new(count.clone()));
            let shiftlight = ArcShift::downgrade(&shift);
            let _shiftlight1 = shiftlight.clone();
            let _shiftlight2 = shiftlight.clone();
            let _shiftlight3 = shiftlight.clone();
            let _shiftlight4 = shiftlight.clone();
            let _shiftlight5 = shiftlight.clone(); //Verify that early drop still happens with several light references (silences a cargo mutants-test :-) )

            debug_println!("== Calling update_shared ==");
            shift.update(InstanceSpy::new(count.clone()));
            assert_eq!(count.load(Ordering::SeqCst), 1);
            debug_println!("== Calling drop(shift) ==");
            drop(shift);
            assert_eq!(count.load(Ordering::SeqCst), 0);
        }
        assert_eq!(count.load(Ordering::Relaxed), 0);
}
#[test]
fn simple_upgrade4c() {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let mut shift = ArcShift::new(InstanceSpy::new(count.clone()));
            let shiftlight = ArcShift::downgrade(&shift);
            let _shiftlight1 = shiftlight.clone();
            let _shiftlight2 = shiftlight.clone();
            let _shiftlight3 = shiftlight.clone();
            let _shiftlight4 = shiftlight.clone();
            let _shiftlight5 = shiftlight.clone(); //Verify that early drop still happens with several light references (silences a cargo mutants-test :-) )

            debug_println!("== Calling update_shared ==");
            shift.update(InstanceSpy::new(count.clone()));
            assert_eq!(count.load(Ordering::Relaxed), 1);
            debug_println!("== Calling drop(shift) ==");
            assert_eq!(count.load(Ordering::Relaxed), 1);
        }
        assert_eq!(count.load(Ordering::Relaxed), 0);
}
}
#[test]
fn simple_threading2() {
    model(|| {
        let shift = ArcShift::new(42u32);
        let mut shift1 = shift.clone();
        let mut shift2 = shift1.clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift1.update(43);
                debug_println!("t1 dropping");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                std::hint::black_box(shift2.get());
                debug_println!("t2 dropping");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
    });
}
#[test]
fn simple_threading2d() {
    model(|| {
        let owner = std::sync::Arc::new(SpyOwner2::new());
        {
            let mut shift = ArcShift::new(owner.create("orig"));
            let shiftlight = ArcShift::downgrade(&shift);
            unsafe { ArcShift::debug_validate(&[&shift], &[&shiftlight]) };
            let t1 = atomic::thread::Builder::new()
                .name("t1".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    black_box(shiftlight.upgrade());
                    debug_println!("t1 dropping");
                })
                .unwrap();

            let ownerref = owner.clone();
            let t2 = atomic::thread::Builder::new()
                .name("t2".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    shift.update(ownerref.create("val1"));
                    shift.update(ownerref.create("val2"));
                    debug_println!("t2 dropping");
                })
                .unwrap();
            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
        }
        owner.validate();
    });
}
#[test]
fn simple_threading_rcu() {
    model(|| {
        let mut shift1 = ArcShift::new(42u32);
        let mut shift2 = ArcShift::clone(&shift1);
        let mut shift3 = ArcShift::clone(&shift1);
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift1.rcu(|prev| *prev + 1);
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift2.rcu(|prev| *prev + 1);
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        assert_eq!(*shift3.get(), 44);
    });
}
#[test]
fn simple_threading2b() {
    model(|| {
        let mut shift2 = ArcShift::new(42u32);
        let shift1 = ArcShift::downgrade(&shift2);
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                black_box(shift1.clone());
                debug_println!("t1 dropping");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                std::hint::black_box(shift2.get());
                debug_println!("t2 dropping");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
    });
}
#[test]
fn simple_threading2c() {
    model(|| {
        let mut shift2 = ArcShift::new(42u32);
        let shift1 = ArcShift::downgrade(&shift2);
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                black_box(shift1.clone());
                debug_println!("=== t1 dropping");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift2.update(43);
                debug_println!("=== t2 dropping");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
    });
}

#[test]
fn simple_threading3a() {
    model(|| {
        let shift1 = std::sync::Arc::new(ArcShift::new(42u32));
        let shift2 = std::sync::Arc::clone(&shift1);
        let mut shift3 = (*shift1).clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                _ = shift1;
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                _ = shift2;
                debug_println!(" = drop t2 =");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t3 =");
                shift3.update(43);
                debug_println!(" = drop t3 =");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        _ = t3.join().unwrap();
    });
}
#[test]
fn simple_threading3b() {
    model(|| {
        let shift1 = std::sync::Arc::new(ArcShift::new(42u32));
        let shift2 = std::sync::Arc::clone(&shift1);
        let mut shift3 = (*shift1).clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                let mut shift = (*shift1).clone();
                std::hint::black_box(shift.get());
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                let mut shift = (*shift2).clone();
                std::hint::black_box(shift.get());
                debug_println!(" = drop t2 =");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t3 =");
                std::hint::black_box(shift3.get());
                debug_println!(" = drop t3 =");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        _ = t3.join().unwrap();
    });
}
#[test]
#[cfg(not(loom))]
fn simple_threading3c() {
    model(|| {
        let shift1 = std::sync::Arc::new(Mutex::new(ArcShift::new(42u32)));
        let shift2 = std::sync::Arc::clone(&shift1);
        let mut shift3 = (shift1.lock().unwrap()).clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                std::hint::black_box(shift1.lock().unwrap().update(43));
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                std::hint::black_box(shift2.lock().unwrap().update(44));
                debug_println!(" = drop t2 =");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t3 =");
                std::hint::black_box(shift3.update(45));
                debug_println!(" = drop t3 =");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        _ = t3.join().unwrap();
    });
}

#[cfg(not(loom))]
#[test]
fn simple_threading3d() {
    model(|| {
        let shift1 = std::sync::Arc::new(Mutex::new(ArcShift::new(42u32)));
        let shift2 = std::sync::Arc::clone(&shift1);
        let shift3 = shift1.clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                std::hint::black_box(shift1.lock().unwrap().reload());
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                std::hint::black_box(shift2.lock().unwrap().update(44));
                debug_println!(" = drop t2 =");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t3 =");
                std::hint::black_box(shift3.lock().unwrap().update(45));
                debug_println!(" = drop t3 =");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        _ = t3.join().unwrap();
    });
}

#[test]
fn simple_threading2_rcu() {
    model(|| {
        let mut shift0 = ArcShift::new(0u32);

        let mut shift1 = shift0.clone();
        let mut shift2 = shift0.clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                shift1.rcu(|old| *old + 1);
                shift1.rcu(|old| *old + 1);
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                shift2.rcu(|old| *old + 1);
                shift2.rcu(|old| *old + 1);
                debug_println!(" = drop t2 =");
            })
            .unwrap();

        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        assert_eq!(*shift0.get(), 4);
    });
}

#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn simple_threading3_rcu() {
    model(|| {
        let mut shift0 = ArcShift::new(0u32);

        let mut shift1 = shift0.clone();
        let mut shift2 = shift0.clone();
        let mut shift3 = shift0.clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                shift1.rcu(|old| *old + 1);
                shift1.rcu(|old| *old + 1);
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                shift2.rcu(|old| *old + 1);
                shift2.rcu(|old| *old + 1);
                debug_println!(" = drop t2 =");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t3 =");
                shift3.rcu(|old| *old + 1);
                shift3.rcu(|old| *old + 1);
                debug_println!(" = drop t3 =");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        _ = t3.join().unwrap();
        assert_eq!(*shift0.get(), 6);
    });
}

#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn simple_threading4a() {
    model(|| {
        let shift1 = std::sync::Arc::new(Mutex::new(ArcShift::new(42u32)));
        let shift2 = std::sync::Arc::clone(&shift1);
        let shift3 = std::sync::Arc::clone(&shift1);
        let mut shift4 = (shift1.lock().unwrap()).clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                shift1.lock().unwrap().update(43);
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                let mut shift = (shift2.lock().unwrap()).clone();
                std::hint::black_box(shift.get());
                debug_println!(" = drop t2 =");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t3 =");
                shift3.lock().unwrap().update(44);
                debug_println!(" = drop t3 =");
            })
            .unwrap();
        let t4 = atomic::thread::Builder::new()
            .name("t4".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t4 =");
                let t = std::hint::black_box(*shift4.get());
                debug_println!(" = drop t4 =");
                t
            })
            .unwrap();

        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        _ = t3.join().unwrap();
        let ret = t4.join().unwrap();
        assert!(ret == 42 || ret == 43 || ret == 44);
    });
}

#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn simple_threading4b() {
    model(|| {
        let shift1 = std::sync::Arc::new(ArcShift::new(42u32));
        let shift2 = std::sync::Arc::clone(&shift1);
        let shift3 = std::sync::Arc::clone(&shift1);
        let shift4 = (*shift1).clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                let mut temp = (*shift1).clone();
                temp.update(43);
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                let mut shift = (*shift2).clone();
                std::hint::black_box(shift.get());
                debug_println!(" = drop t2 =");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t3 =");
                let mut temp = (*shift3).clone();
                temp.update(44);
                let t = std::hint::black_box(temp.get());
                debug_println!(" = drop t3 =");
                return *t;
            })
            .unwrap();
        let t4 = atomic::thread::Builder::new()
            .name("t4".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t4 =");
                let t = std::hint::black_box(shift4.try_into_inner());
                debug_println!(" = drop t4 =");
                t
            })
            .unwrap();

        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        let ret3 = t3.join().unwrap();
        assert!(ret3 == 44 || ret3 == 43);
        let ret = t4.join().unwrap();
        assert!(ret == None || ret == Some(43) || ret == Some(44) || ret == Some(42));
    });
}
#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn simple_threading4c() {
    model(|| {
        let count = std::sync::Arc::new(SpyOwner2::new());
        {
            let shift1 =
                std::sync::Arc::new(atomic::Mutex::new(ArcShift::new(count.create("orig"))));
            let shift2 = std::sync::Arc::clone(&shift1);
            let shift3 = std::sync::Arc::clone(&shift1);
            let shift4 = std::sync::Arc::clone(&shift1);
            let _t = std::sync::Arc::clone(&shift1);
            let count1 = count.clone();
            let count2 = count.clone();
            let t1 = atomic::thread::Builder::new()
                .name("t1".to_string())
                .stack_size(1_000_00)
                .spawn(move || {
                    debug_println!(" = On thread t1 = {:?}", std::thread::current().id());
                    shift1.lock().unwrap().update(count1.create("t1val"));
                    debug_println!(" = drop t1 =");
                })
                .unwrap();

            let t2 = atomic::thread::Builder::new()
                .name("t2a".to_string())
                .stack_size(1_000_00)
                .spawn(move || {
                    debug_println!(" = On thread t2 = {:?}", std::thread::current().id());
                    let mut _shift = shift2; //.clone()

                    debug_println!(" = drop t2 =");
                })
                .unwrap();

            let t3 = atomic::thread::Builder::new()
                .name("t3".to_string())
                .stack_size(1_000_00)
                .spawn(move || {
                    debug_println!(" = On thread t3 = {:?}", std::thread::current().id());
                    shift3.lock().unwrap().update(count2.create("t3val"));

                    debug_println!(" = drop t3 =");
                })
                .unwrap();
            let t4 = atomic::thread::Builder::new()
                .name("t4".to_string())
                .stack_size(1_000_00)
                .spawn(move || {
                    debug_println!(" = On thread t4 = {:?}", std::thread::current().id());
                    let shift4 = &*shift4;
                    //verify_item(shift4.item.as_ptr());
                    let _t = std::hint::black_box(shift4);
                    debug_println!(" = drop t4 =");
                })
                .unwrap();

            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            _ = t3.join().unwrap();
            _ = t4.join().unwrap();
        }
        debug_println!("All threads stopped");
        count.validate();
    });
}

#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn simple_threading4d() {
    model(|| {
        let count = std::sync::Arc::new(SpyOwner2::new());
        {
            let count3 = count.clone();
            let shift1 = std::sync::Arc::new(ArcShift::new(count.create("orig")));

            let shift2 = shift1.clone();
            let shift3 = shift1.clone();
            let shift4 = shift1.clone();
            let t1 = atomic::thread::Builder::new()
                .name("t1".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    debug_println!(" = On thread t1 =");
                    black_box(shift1.clone());
                    debug_println!(" = drop t1 =");
                })
                .unwrap();

            let t2 = atomic::thread::Builder::new()
                .name("t2".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    debug_println!(" = On thread t2 =");
                    let mut shift = (*shift2).clone();
                    std::hint::black_box(shift.get());
                    debug_println!(" = drop t2 =");
                })
                .unwrap();

            let t3 = atomic::thread::Builder::new()
                .name("t3".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    debug_println!(" = On thread t3 =");
                    let mut temp = (*shift3).clone();
                    temp.update(count3.create("t3val"));
                    let _t = std::hint::black_box(shift3.shared_get());
                    debug_println!(" = drop t3 =");
                })
                .unwrap();
            let t4 = atomic::thread::Builder::new()
                .name("t4".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    debug_println!(" = On thread t4 =");
                    let mut temp = (*shift4).clone();
                    let _t = std::hint::black_box(temp.try_get_mut());
                    debug_println!(" = drop t4 =");
                })
                .unwrap();

            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            _ = t3.join().unwrap();
            _ = t4.join().unwrap();
            debug_println!("JOined all threads");
            atomic::loom_fence();
        }

        count.validate();
        drop(count);
    });
}

#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn simple_threading4e() {
    model(|| {
        let shift = std::sync::Arc::new(Mutex::new(ArcShift::new(42u32)));
        let shift1 = shift.clone();
        let shift2 = shift.clone();
        let shift3 = shift.clone();
        let shift4 = shift.clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                shift1.lock().unwrap().update(43);
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                shift2.lock().unwrap().update(44);
                debug_println!(" = drop t2 =");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t3 =");
                shift3.lock().unwrap().update(45);
                debug_println!(" = drop t3 =");
            })
            .unwrap();
        let t4 = atomic::thread::Builder::new()
            .name("t4".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t4 =");
                shift4.lock().unwrap().update(46);
                debug_println!(" = drop t4 =");
            })
            .unwrap();

        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        _ = t3.join().unwrap();
        _ = t4.join().unwrap();
    });
}

#[test]
fn simple_test_clones2() {
    model(|| {
        let shift = ArcShift::new("orig".to_string());
        let shift1 = ArcShift::downgrade(&shift);
        let shift2 = shift.clone();
        let shift3 = shift.clone();
        unsafe { ArcShift::debug_validate(&[&shift, &shift2, &shift3], &[&shift1]) };
    });
}
#[test]
fn simple_test_clonesp2() {
    model(|| {
        let _shift = ArcShift::new("orig".to_string());
    })
}
#[test]
fn simple_threading_update_in_one() {
    model(|| {
        debug_println!("-------- loom -------------");
        let mut shift = ArcShift::new(42u32);
        let mut shift1 = shift.clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift.update(43);
                debug_println!("t1 dropping");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                std::hint::black_box(shift1.get());
                debug_println!("t2 dropping");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
    });
}
#[test]
fn simple_threading_repro1() {
    model(|| {
        debug_println!("-------- loom -------------");
        let root = ArcShift::new(42u32);
        let mut curval = root.clone();
        let light = ArcShift::downgrade(&curval);
        unsafe { ArcShift::debug_validate(&[&root, &curval], &[&light]) };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                curval.update(42);
                debug_println!("t1 dropping");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                let _ = light.upgrade();
                debug_println!("t2 dropping");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();

        unsafe { ArcShift::debug_validate(&[&root], &[]) };
    });
}
#[test]
fn simple_threading_repro3() {
    model(|| {
        debug_println!("-------- loom -------------");
        let root = ArcShift::new(42u32);
        let arc1 = ArcShift::downgrade(&root);
        let arc2 = ArcShift::downgrade(&root);
        drop(root);
        unsafe { ArcShift::debug_validate(&[], &[&arc1, &arc2]) };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                let _x = arc1;
                debug_println!("t1 dropping");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                let _x = arc2;
                debug_println!("t2 dropping");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
    });
}
#[test]
fn simple_threading_repro2() {
    model(|| {
        debug_println!("-------- loom -------------");
        let root = ArcShift::new(42u32);
        let mut curval = root.clone();
        let light = ArcShift::downgrade(&curval);
        curval.update(42);
        debug_println!("----> curval.dropping");
        drop(curval);

        println!("----> light.upgrade");
        light.upgrade();
        unsafe { ArcShift::debug_validate(&[&root], &[&light]) };
        drop(light);
    });
}

#[test]
fn simple_threading_update_twice() {
    model(|| {
        debug_println!("-------- loom -------------");
        let mut shift = ArcShift::new(42u32);
        let mut shift1 = shift.clone();
        let mut shift2 = shift.clone();
        unsafe { ArcShift::debug_validate(&[&shift, &shift1, &shift2], &[]) };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift.update(43);
                debug_println!("--> t1 dropping");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift1.update(44);
                debug_println!("--> t2 dropping");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        unsafe { ArcShift::debug_validate(&[&shift2], &[]) };
        debug_println!("--> Main dropping");
        assert!(*shift2.get() > 42);
    });
}
#[test]
fn simple_threading_update_thrice() {
    model(|| {
        debug_println!("-------- loom -------------");
        let mut shift = ArcShift::new(42u32);
        let mut shift1 = shift.clone();
        let mut shift2 = shift.clone();
        let mut shift3 = shift.clone();
        unsafe { ArcShift::debug_validate(&[&shift, &shift1, &shift2, &shift3], &[]) };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift.update(43);
                debug_println!("--> t1 dropping");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift1.update(44);
                debug_println!("--> t2 dropping");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift2.update(45);
                debug_println!("--> t3 dropping");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        _ = t3.join().unwrap();
        unsafe { ArcShift::debug_validate(&[&shift3], &[]) };
        debug_println!("--> Main dropping");
        assert!(*shift3.get() > 42);
    });
}
