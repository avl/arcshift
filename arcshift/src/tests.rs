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
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::mem::MaybeUninit;

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
const SHUTTLE_ITERATIONS: usize = 50;

#[cfg(feature = "shuttle")]
fn model(x: impl Fn() + 'static + Send + Sync) {
    shuttle::check_random(x, SHUTTLE_ITERATIONS);
}
#[cfg(feature = "shuttle")]
fn model2(x: impl Fn() + 'static + Send + Sync, repro: Option<&str>) {
    if let Some(repro) = repro {
        shuttle::replay(x, repro);
    } else {
        shuttle::check_random(x, SHUTTLE_ITERATIONS);
    }
}

// Here follows some simple basic tests

#[test]
fn simple_get() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
    })
}
#[test]
fn simple_box() {
    model(|| {
        let mut shift = ArcShift::from_box(Box::new(42u32));
        assert_eq!(*shift.get(), 42u32);
    })
}
#[test]
fn simple_unsized() {
    model(|| {
        let biggish = vec![1u32, 2u32].into_boxed_slice();
        let mut shift = ArcShift::from_box(biggish);
        debug_println!("Drop");
        assert_eq!(shift.get(), &vec![1, 2]);
    })
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
    model(|| {
        let boxed_trait: Box<dyn ExampleTrait> = Box::new(ExampleStruct { x: 42 });
        let mut shift = ArcShift::from_box(boxed_trait);
        debug_println!("Drop");
        assert_eq!(shift.get().call(), 42);
    })
}
#[test]
fn simple_unsized_str() {
    model(|| {
        let boxed_str: Box<str> = Box::new("hello".to_string()).into_boxed_str();
        let mut shift = ArcShift::from_box(boxed_str);
        debug_println!("Drop");
        assert_eq!(shift.get(), "hello");
    })
}
use std::cell::{Cell, RefCell};
use crate::cell::ArcShiftCell;

thread_local! {

    static THREADLOCAL_FOO: ArcShiftCell<String> = ArcShiftCell::new(String::new());
}

#[cfg(not(any(loom, feature = "shuttle")))]
//This test doesn't work in shuttle or loom, since the lazy drop of the threadlocal ends up happening outside of the shuttle model
#[test]
fn simple_threadlocal_cell() {
    model(|| {
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
    })
}

#[test]
fn simple_cell() {
    model(|| {
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
    });
}
#[test]
fn simple_cell_handle() {
    model(|| {
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
    });
}

#[test]
fn simple_multiple_cell_handles() {
    model(|| {
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
    });
}

#[test]
fn simple_cell_recursion() {
    model(|| {
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
            cell.get(|val|{
                assert_eq!(val.str(), "B");
            });
        }
        owner.validate();
    });
}
#[test]
fn simple_cell_assign() {
    model(|| {
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
    });
}

//TODO: Implement and test rcu
/*
#[test]
fn simple_rcu() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        assert!(shift.rcu(|x| x + 1));
        assert_eq!(*shift.get(), 43u32);
    })
}
#[test]
fn simple_rcu_project() {
    model(|| {
        let mut shift = ArcShift::new((1u32, 10u32));
        assert_eq!(
            shift.rcu_project(|(a, b)| Some((*a + 1, *b + 1)), |(a, _b)| a),
            (true, &2u32)
        );
    })
}
#[test]
fn simple_rcu_project2() {
    model(|| {
        let mut shift = ArcShift::new((1u32, 10u32));
        assert_eq!(
            shift.rcu_project(|(_a, _b)| None, |(a, _b)| a),
            (false, &1u32)
        );
    })
}
#[cfg(not(any(feature = "shuttle", loom)))]
#[test]
fn simple_rcu_project3() {
    let outerstuff = "hej".to_string();
    let outer = outerstuff.as_str();
    let mut escape = None;
    model(|| {
        let mut shift = ArcShift::new((1u32, 10u32));
        escape = Some(shift.rcu_project(|(_a, _b)| None, |(_a, _b)| outer).1);
        debug_println!("Escaped: {:?}", escape);
        debug_println!("Shift get: {:?}", shift.get());
    });
    debug_println!("Escaped: {:?}", escape);
}

#[test]
fn simple_rcu_maybe() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        assert!(shift.rcu_maybe(|x| Some(x + 1)));
        assert_eq!(*shift.get(), 43u32);
        assert_eq!(shift.rcu_maybe(|_x| None), false);
        assert_eq!(*shift.get(), 43u32);
    })
}
#[test]
fn simple_rcu_safe() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        shift.rcu_safe(|x| x + 1);
        assert_eq!(*shift.get(), 43u32);
    })
}
#[test]
fn simple_rcu_maybe2() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(shift.rcu_maybe2(|x| Some(x + 1)), RcuResult::Update);
        assert_eq!(shift.rcu_maybe2(|_x| None), RcuResult::NoUpdate);
        assert_eq!(*shift.get(), 43u32);
        assert_eq!(shift.rcu_maybe(|_x| None), false);
        assert_eq!(*shift.get(), 43u32);
    })
}
 */

#[test]
fn simple_deref() {
    model(|| {
        let shift = ArcShift::new(42u32);
        assert_eq!(*shift, 42u32);
    })
}
#[test]
fn simple_get4() {
    model(|| {
        let shift = ArcShift::new(42u32);
        assert_eq!(*shift.shared_get(), 42u32);
    })
}
/* TODO: Implement get_mut
#[test]
fn simple_get_mut() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        // Uniquely owned values can be modified using 'try_get_mut'.
        assert_eq!(*shift.try_get_mut().unwrap(), 42);
    })
}*/
#[test]
fn simple_zerosized() {
    model(|| {
        let mut shift = ArcShift::new(());
        assert_eq!(*shift.get(), ());
        shift.update(());
        assert_eq!(*shift.get(), ());
    })
}
#[test]
fn simple_update() {
    model(|| {
        let mut shift = ArcShift::new(42);
        let old = &*shift;
        shift.update(*old + 4);
        shift.reload();
    })
}
/*
TODO: implement get_mut and try_into_inner
#[test]
fn simple_get_mut2() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        let mut shift2 = shift.clone();
        shift2.update(43);
        assert_eq!(shift.try_get_mut(), None);
    })
}

#[test]
fn simple_try_into() {
    model(|| {
        let shift = ArcShift::new(42u32);
        // Uniquely owned values can be moved out without being dropped
        assert_eq!(shift.try_into_inner().unwrap(), 42);
    })
}
*/


#[test]
// There's no point in running this test under shuttle/loom,
// and since it can take some time, let's just disable it.
#[cfg(not(any(loom, feature = "shuttle")))]
fn simple_large() {
    model(|| {
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
    })
}
#[test]
fn simple_get2() {
    model(|| {
        let mut shift = ArcShift::from_box(Box::new(42u32));
        assert_eq!(*shift.get(), 42u32);
    })
}
#[test]
fn simple_get3() {
    model(|| {
        let mut shift = ArcShift::new("hello".to_string());
        assert_eq!(shift.get(), "hello");
    })
}
#[test]
fn simple_update0() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
        shift.update(43);
        assert_eq!(*shift.get(), 43u32);
    });
}
#[test]
fn simple_update_boxed() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
        shift.update_box(Box::new(43));
        assert_eq!(*shift.get(), 43u32);
    });
}

#[test]
fn simple_update2() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
        shift.update(43);
        shift.update(44);
        shift.update(45);
        assert_eq!(*shift.get(), 45u32);
    });
}
#[test]
fn simple_update3() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
        shift.update_box(Box::new(45));
        assert_eq!(*shift.get(), 45u32);
    });
}

#[test]
fn simple_update5() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
        shift.update(45);
        assert_eq!(*shift.get(), 45u32);
    });
}

#[test]
fn simple_upgrade3a1() {
    model(|| {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let shift1 = ArcShift::new(InstanceSpy::new(count.clone()));
        let shiftlight = ArcShift::downgrade(&shift1);

        debug_println!("==== running shift.get() = ");
        let mut shift2 = shiftlight.upgrade().unwrap();
        debug_println!("==== running arc.update() = ");
        shift2.update(InstanceSpy::new(count.clone()));

        unsafe { ArcShift::debug_validate(&[&shift1,&shift2], &[&shiftlight]) };
        debug_println!("==== Instance count: {}", count.load(Ordering::SeqCst));
        drop(shift1);
        assert_eq!(count.load(Ordering::SeqCst), 1); // The 'ArcShiftLight' should *not* keep any version alive
        debug_println!("==== drop arc =");
        drop(shift2);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        debug_println!("==== drop shiftroot =");
        drop(shiftlight);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    });
}
#[test]
fn simple_upgrade3a0() {
    model(|| {
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
    });
}

#[test]
fn simple_upgrade4b() {
    model(|| {
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
            drop(shift);
            assert_eq!(count.load(Ordering::Relaxed), 1);
        }
        assert_eq!(count.load(Ordering::Relaxed), 0);
    });
}
#[test]
fn simple_upgrade4c() {
    model(|| {
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
    });
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
            unsafe {ArcShift::debug_validate(&[&shift],&[&shiftlight]) };
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
/* TODO: impl rcu
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
                shift1.rcu_safe(|prev| *prev + 1);
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift2.rcu_safe(|prev| *prev + 1);
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        assert_eq!(*shift3.get(), 44);
    });
}
*/
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

/* TODO: implement rcu
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
                while !shift1.rcu(|old| *old + 1) {}
                while !shift1.rcu(|old| *old + 1) {}
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                while !shift2.rcu(|old| *old + 1) {}
                while !shift2.rcu(|old| *old + 1) {}
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
                while !shift1.rcu(|old| *old + 1) {}
                while !shift1.rcu(|old| *old + 1) {}
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                while !shift2.rcu(|old| *old + 1) {}
                while !shift2.rcu(|old| *old + 1) {}
                debug_println!(" = drop t2 =");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t3 =");
                while !shift3.rcu(|old| *old + 1) {}
                while !shift3.rcu(|old| *old + 1) {}
                debug_println!(" = drop t3 =");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        _ = t3.join().unwrap();
        assert_eq!(*shift0.get(), 6);
    });
}
 */
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

/* TODO implement try_into_inner
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
                shift1.update(43);
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
                shift3.update(44);
                let t = std::hint::black_box((*shift3).shared_get());
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
*/
#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn simple_threading4c() {
    model(|| {
        let count = std::sync::Arc::new(SpyOwner2::new());
        {
            let shift1 = std::sync::Arc::new(Mutex::new(ArcShift::new(count.create("orig"))));
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

/* TODO: implement try_get_mut
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
            let mut shift4 = shift1.clone();
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
                    let mut shift = shift2.clone();
                    std::hint::black_box(shift.get());
                    debug_println!(" = drop t2 =");
                })
                .unwrap();

            let t3 = atomic::thread::Builder::new()
                .name("t3".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    debug_println!(" = On thread t3 =");
                    shift3.update(count3.create("t3val"));
                    let _t = std::hint::black_box(shift3.shared_get());
                    debug_println!(" = drop t3 =");
                })
                .unwrap();
            let t4 = atomic::thread::Builder::new()
                .name("t4".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    debug_println!(" = On thread t4 =");
                    let _t = std::hint::black_box(shift4.try_get_mut());
                    debug_println!(" = drop t4 =");
                })
                .unwrap();

            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            _ = t3.join().unwrap();
            _ = t4.join().unwrap();
            debug_println!("JOined all threads");
            atomic::fence(Ordering::SeqCst);
        }

        count.validate();
        drop(count);
    });
}
*/

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
