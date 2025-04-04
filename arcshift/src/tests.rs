#![deny(warnings)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(clippy::bool_assert_comparison)]
#![allow(clippy::unit_cmp)]
use super::*;
use crate::cell::ArcShiftCell;
use leak_detection::{InstanceSpy, InstanceSpy2, SpyOwner2};
use rand::prelude::StdRng;
use rand::{Rng, SeedableRng};
use std::alloc::Layout;
use std::boxed::Box;
use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::hint::black_box;
use std::mem::MaybeUninit;
use std::string::ToString;
use std::sync::atomic::AtomicUsize;
use std::thread;
use std::time::Duration;
use std::vec;

mod custom_fuzz;
pub(crate) mod leak_detection;
mod race_detector;

// All tests are wrapped by these 'model' calls.
// This is needed to make the tests runnable from within the Shuttle and Loom frameworks.

#[cfg(all(not(loom), not(feature = "shuttle")))]
pub(crate) fn model(x: impl FnOnce()) {
    x()
}
#[cfg(all(not(loom), not(feature = "shuttle")))]
fn model2(x: impl FnOnce(), _repro: Option<&str>) {
    x()
}
#[cfg(all(not(loom), not(feature = "shuttle")))]
pub(crate) fn dummy_model(x: impl FnOnce()) {
    x()
}

#[cfg(loom)]
pub(crate) fn model(x: impl Fn() + 'static + Send + Sync) {
    loom::model(x)
}
#[cfg(loom)]
fn model2(x: impl Fn() + 'static + Send + Sync, _repro: Option<&str>) {
    loom::model(x)
}
#[cfg(loom)]
pub(crate) fn dummy_model(x: impl Fn() + 'static + Send + Sync) {
    loom::model(x)
}

#[cfg(all(feature = "shuttle", coverage))]
const SHUTTLE_ITERATIONS: usize = 50;
#[cfg(all(feature = "shuttle", not(coverage)))]
const SHUTTLE_ITERATIONS: usize = 500000;

#[cfg(feature = "shuttle")]
pub(crate) fn model(x: impl Fn() + 'static + Send + Sync) {
    shuttle::check_pct(x, SHUTTLE_ITERATIONS, 4);
}
#[cfg(feature = "shuttle")]
pub(crate) fn dummy_model(x: impl Fn() + 'static + Send + Sync) {
    shuttle::check_random(x, 1);
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
mod simple {
    use super::{dummy_model, model, SpyOwner2};
    use std::alloc::Layout;
    use std::string::ToString;
    use std::vec;

    use crate::{
        decorate, get_decoration, get_weak_count, get_weak_next, get_weak_prev, ArcShift,
        ItemStateEnum, SizedMetadata,
    };
    use alloc::boxed::Box;

    #[test]
    fn simple_get() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            assert_eq!(*shift.get(), 42u32);
        });
    }
    #[test]
    fn simple_box() {
        dummy_model(|| {
            let mut shift = ArcShift::from_box(Box::new(42u32));
            assert_eq!(*shift.get(), 42u32);
        });
    }
    #[test]
    fn simple_unsized() {
        dummy_model(|| {
            let biggish = vec![1u32, 2u32].into_boxed_slice();
            let mut shift = ArcShift::from_box(biggish);
            debug_println!("Drop");
            assert_eq!(shift.get(), &vec![1, 2]);
        });
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
        dummy_model(|| {
            let boxed_trait: Box<dyn ExampleTrait> = Box::new(ExampleStruct { x: 42 });
            let mut shift = ArcShift::from_box(boxed_trait);
            debug_println!("Drop");
            assert_eq!(shift.get().call(), 42);
        });
    }
    #[test]
    fn simple_unsized_str() {
        dummy_model(|| {
            let boxed_str: Box<str> = Box::new("hello".to_string()).into_boxed_str();
            let mut shift = ArcShift::from_box(boxed_str);
            debug_println!("Drop");
            assert_eq!(shift.get(), "hello");
        });
    }
    use crate::cell::ArcShiftCell;
    use crate::tests::leak_detection::InstanceSpy;
    use std::cell::{Cell, RefCell};
    use std::mem::MaybeUninit;
    use std::sync::atomic::Ordering;

    std::thread_local! {

        static THREADLOCAL_FOO: ArcShiftCell<std::string::String> = ArcShiftCell::new(std::string::String::new());
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
        dummy_model(|| {
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
        dummy_model(|| {
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
        dummy_model(|| {
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
                    assert_eq!(root.get().str(), "C");
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
        dummy_model(|| {
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
        });
    }
    #[test]
    fn simple_cell_assign() {
        dummy_model(|| {
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

    #[test]
    fn simple_rcu() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            shift.rcu(|x| x + 1);
            assert_eq!(*shift.get(), 43u32);
        });
    }

    #[test]
    fn simple_rcu_maybe() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            assert!(shift.rcu_maybe(|x| Some(x + 1)));
            assert_eq!(*shift.get(), 43u32);
            assert_eq!(shift.rcu_maybe(|_x| None), false);
            assert_eq!(*shift.get(), 43u32);
        });
    }
    #[test]
    fn simple_rcu_maybe2() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            assert_eq!(shift.rcu_maybe(|x| Some(x + 1)), true);
            assert_eq!(shift.rcu_maybe(|_x| None), false);
            assert_eq!(*shift.get(), 43u32);
            assert_eq!(shift.rcu_maybe(|_x| None), false);
            assert_eq!(*shift.get(), 43u32);
        });
    }

    #[test]
    fn simple_rcu_maybe3() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            let mut shift2 = shift.clone();
            assert_eq!(shift.rcu_maybe(|x| Some(x + 1)), true);
            assert_eq!(shift.rcu_maybe(|_x| None), false);
            assert_eq!(*shift.get(), 43u32);
            assert_eq!(shift.rcu_maybe(|_x| None), false);
            assert_eq!(*shift.get(), 43u32);
            assert_eq!(*shift2.get(), 43u32);
        });
    }
    #[test]
    fn simple_deref() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            assert_eq!(*shift.get(), 42u32);
        });
    }
    #[test]
    fn simple_get4() {
        dummy_model(|| {
            let shift = ArcShift::new(42u32);
            assert_eq!(*shift.shared_non_reloading_get(), 42u32);
        });
    }
    #[test]
    fn simple_get5() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            let shift2 = shift.clone();
            shift.update(43);

            assert_eq!(*shift2.shared_get(), 43u32);
        });
    }

    #[test]
    fn simple_get_mut() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            // Uniquely owned values can be modified using 'try_get_mut'.
            assert_eq!(*shift.try_get_mut().unwrap(), 42);
        });
    }
    #[test]
    fn simple_zerosized() {
        dummy_model(|| {
            let mut shift = ArcShift::new(());
            assert_eq!(*shift.get(), ());
            shift.update(());
            assert_eq!(*shift.get(), ());
        });
    }
    #[test]
    fn simple_update() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42);
            let old = *shift.get();
            shift.update(old + 4);
            shift.reload();
        });
    }

    #[test]
    fn simple_get_mut2() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            let mut shift2 = shift.clone();
            shift2.update(43);
            assert_eq!(shift.try_get_mut(), None);
        });
    }
    #[test]
    fn simple_get_mut3() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            let _shift2 = ArcShift::downgrade(&shift);
            shift.update(43);
            assert_eq!(shift.try_get_mut(), None);
        });
    }
    #[test]
    fn simple_get_mut4_mut() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            let mut shift2 = shift.clone();
            shift2.update(43);
            drop(shift2);
            assert_eq!(shift.try_get_mut(), Some(&mut 43));
        });
    }
    #[test]
    fn simple_try_into() {
        dummy_model(|| {
            let shift = ArcShift::new(42u32);
            // Uniquely owned values can be moved out without being dropped
            assert_eq!(shift.try_into_inner().unwrap(), 42);
        });
    }

    #[test]
    fn simple_into_inner() {
        dummy_model(|| {
            let shift = ArcShift::new(42u32);
            // Uniquely owned values can be moved out without being dropped
            assert_eq!(shift.try_into_inner().unwrap(), 42);
        });
    }
    #[test]
    fn simple_into_inner2() {
        dummy_model(|| {
            let shift = ArcShift::new(42u32);
            let mut shift2 = shift.clone();
            shift2.update(43);
            drop(shift2);
            assert_eq!(shift.try_into_inner(), Some(43));
        });
    }
    #[test]
    fn simple_failing_into_inner() {
        dummy_model(|| {
            let shift = ArcShift::new(42u32);
            let _shift2 = shift.clone();

            assert_eq!(shift.try_into_inner(), None);
        });
    }
    #[test]
    fn simple_failing_into_inner2() {
        dummy_model(|| {
            let shift = ArcShift::new(42u32);
            let mut shift2 = shift.clone();
            shift2.update(43);

            assert_eq!(shift.try_into_inner(), None);
        });
    }

    #[test]
    // There's no point in running this test under shuttle/loom,
    // and since it can take some time, let's just disable it.
    #[cfg(not(any(loom, feature = "shuttle")))]
    fn simple_large() {
        dummy_model(|| {
            #[cfg(not(miri))]
            const SIZE: usize = 10_000_000;
            #[cfg(miri)]
            const SIZE: usize = 10_000;

            let layout = Layout::new::<MaybeUninit<[u64; SIZE]>>();

            let ptr: *mut [u64; SIZE] =
            // # SAFETY:
            // layout is not zero-sized
            unsafe { std::alloc::alloc(layout) } as *mut [u64; SIZE];
            for i in 0..SIZE {
                // # SAFETY:
                // ptr is valid (just allocated)
                unsafe { *(*ptr).get_unchecked_mut(i) = 42 };
            }
            // # SAFETY:
            // ptr is valid (just allocated)
            let bigbox = unsafe { Box::from_raw(ptr) };
            let mut shift = ArcShift::from_box(bigbox);
            let value = shift.get();
            assert_eq!(value[0], 42);
        });
    }
    #[test]
    fn simple_get2() {
        dummy_model(|| {
            let mut shift = ArcShift::from_box(Box::new(42u32));
            assert_eq!(*shift.get(), 42u32);
        });
    }
    #[test]
    fn simple_get3() {
        dummy_model(|| {
            let mut shift = ArcShift::new("hello".to_string());
            assert_eq!(shift.get(), "hello");
        });
    }
    #[test]
    fn simple_update0() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            assert_eq!(*shift.get(), 42u32);
            shift.update(43);
            assert_eq!(*shift.get(), 43u32);
        });
    }
    #[test]
    fn simple_update_boxed() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            assert_eq!(*shift.get(), 42u32);
            shift.update_box(Box::new(43));
            assert_eq!(*shift.get(), 43u32);
        });
    }

    #[test]
    fn simple_update2() {
        dummy_model(|| {
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
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            assert_eq!(*shift.get(), 42u32);
            shift.update_box(Box::new(45));
            assert_eq!(*shift.get(), 45u32);
        });
    }

    #[test]
    fn simple_update5() {
        dummy_model(|| {
            let mut shift = ArcShift::new(42u32);
            assert_eq!(*shift.get(), 42u32);
            shift.update(45);
            assert_eq!(*shift.get(), 45u32);
        });
    }

    #[test]
    fn simple_upgrade3a1() {
        dummy_model(|| {
            let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let shift1 = ArcShift::new(InstanceSpy::new(count.clone()));
            let shiftlight = ArcShift::downgrade(&shift1);

            debug_println!("==== running shift.get() = ");
            let mut shift2 = shiftlight.upgrade().unwrap();
            debug_println!("==== running arc.update() = ");
            shift2.update(InstanceSpy::new(count.clone()));

            // SAFETY:
            // No threading involved
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
        });
    }
    #[test]
    fn simple_upgrade3a0() {
        dummy_model(|| {
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
        dummy_model(|| {
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
        });
    }
    #[test]
    fn simple_upgrade4c() {
        dummy_model(|| {
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
    fn simple_create() {
        dummy_model(|| {
            let mut x = ArcShift::new(Box::new(45u32));
            assert_eq!(x.strong_count(), 1);
            assert_eq!(x.weak_count(), 1);
            assert_eq!(**x.get(), 45);
            // SAFETY:
            // No threading involved, &x is valid.
            unsafe { ArcShift::debug_validate(&[&x], &[]) };
        });
    }

    #[test]
    fn simple_janitor_test() {
        dummy_model(|| {
            let mut x = ArcShift::new(45u32);
            x.update(46);
            // SAFETY:
            // No threading involved, item pointer of ArcShift is always valid
            let item = unsafe { &*crate::from_dummy::<u32, SizedMetadata>(x.item.as_ptr()) };
            let prev = item.prev.load(crate::atomic::Ordering::SeqCst);
            assert_eq!(prev, core::ptr::null_mut());
            assert_eq!(*x.get(), 46);
        });
    }

    #[test]
    fn simple_create_and_update_once() {
        dummy_model(|| {
            let mut x = ArcShift::new(Box::new(45u32));
            assert_eq!(**x.get(), 45);
            x.update(Box::new(1u32));
            assert_eq!(**x.get(), 1);
            // SAFETY:
            // No threading involved, &x is valid.
            unsafe { ArcShift::debug_validate(&[&x], &[]) };
        });
    }
    #[test]
    fn simple_create_and_update_twice() {
        dummy_model(|| {
            let mut x = ArcShift::new(Box::new(45u32));
            assert_eq!(**x.get(), 45);
            x.update(Box::new(1u32));
            assert_eq!(**x.get(), 1);
            x.update(Box::new(21));
            assert_eq!(**x.get(), 21);
            // SAFETY:
            // No threading involved, &x is valid.
            unsafe { ArcShift::debug_validate(&[&x], &[]) };
        });
    }
    #[test]
    fn simple_create_and_clone() {
        dummy_model(|| {
            let mut x = ArcShift::new(Box::new(45u32));
            let mut y = x.clone();
            assert_eq!(**x.get(), 45);
            assert_eq!(**y.get(), 45);
            // SAFETY:
            // No threading involved, &x and &y are valid.
            unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
        });
    }
    #[test]
    fn simple_create_and_clone_drop_other_order() {
        dummy_model(|| {
            let mut x = ArcShift::new(Box::new(45u32));
            let mut y = x.clone();
            assert_eq!(**x.get(), 45);
            assert_eq!(**y.get(), 45);
            // SAFETY:
            // No threading involved, &x and &y are valid.
            unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
            drop(x);
            drop(y);
        });
    }
    #[test]
    fn simple_downgrade() {
        dummy_model(|| {
            let x = ArcShift::new(Box::new(45u32));
            let _y = ArcShift::downgrade(&x);
        });
    }
    #[test]
    fn pointer_decoration() {
        dummy_model(|| {
            assert_eq!(
                get_decoration(decorate::<u32>(
                    1 as *mut _,
                    ItemStateEnum::UndisturbedGcIsActive
                )),
                ItemStateEnum::UndisturbedGcIsActive
            );
        });
    }
    #[test]
    #[should_panic(expected = "panic: A")]
    fn panic_boxed_drop() {
        dummy_model(|| {
            struct PanicOnDrop(char);
            impl Drop for PanicOnDrop {
                fn drop(&mut self) {
                    panic!("panic: {}", self.0)
                }
            }
            _ = Box::new(PanicOnDrop('A'));
        });
    }
    #[test]
    #[should_panic(expected = "panic: A")]
    fn simple_panic() {
        dummy_model(|| {
            struct PanicOnDrop(char);
            impl Drop for PanicOnDrop {
                fn drop(&mut self) {
                    panic!("panic: {}", self.0)
                }
            }
            // Use a box so that T has a heap-allocation, so miri will tell us
            // if it's dropped correctly (it should be)
            let a = ArcShift::new(Box::new(PanicOnDrop('A')));
            let mut b = a.clone();
            b.update(Box::new(PanicOnDrop('B')));
            drop(b); //This doesn't drop anything, since 'b' is kept alive by next-ref of a
            drop(a); //This will panic, but shouldn't leak memory
        });
    }
    #[test]
    fn simple_create_and_clone_and_update1() {
        dummy_model(|| {
            let mut right = ArcShift::new(Box::new(1u32));
            assert_eq!(right.strong_count(), 1);
            assert_eq!(right.weak_count(), 1);
            let left = right.clone();
            right.update(Box::new(2u32)); // becomes right here
            assert_eq!(right.strong_count(), 1);
            assert_eq!(right.weak_count(), 1);
            assert_eq!(**right.get(), 2);
            assert_eq!(**left.shared_non_reloading_get(), 1);
            assert_eq!(left.strong_count(), 1);
            assert_eq!(left.weak_count(), 2); //'left' and ref from right
            debug_println!("Dropping 'left'");
            // SAFETY:
            // No threading involved, &left and &right are valid.
            unsafe { ArcShift::debug_validate(&[&left, &right], &[]) };
            drop(left);
            assert_eq!(right.strong_count(), 1);
            assert_eq!(right.weak_count(), 1);
        });
    }
    #[test]
    fn get_weak_count_works() {
        dummy_model(|| {
            assert_eq!(get_weak_count(1 << 55), 1 << 55);
            assert_eq!(get_weak_count(1), 1);
            assert_eq!(get_weak_count(542 | (1 << 62)), 542);
            assert_eq!(get_weak_count(542 | (1 << 63)), 542);
            assert_eq!(get_weak_count(1 << 62), 0);
            assert_eq!(get_weak_count(1 << 63), 0);
            assert!(get_weak_next(1 << 63));
            assert!(!get_weak_prev(1 << 63));
            assert!(!get_weak_next(1 << 62));
            assert!(get_weak_prev(1 << 62));
        });
    }
    #[test]
    fn simple_create_and_clone_and_update_other_drop_order() {
        dummy_model(|| {
            let mut x = ArcShift::new(Box::new(1u32));
            assert_eq!(x.strong_count(), 1);
            assert_eq!(x.weak_count(), 1);
            let y = x.clone();
            assert_eq!(y.strong_count(), 2);
            assert_eq!(y.weak_count(), 1);
            x.update(Box::new(2u32));
            assert_eq!(x.strong_count(), 1);
            assert_eq!(x.weak_count(), 1);

            assert_eq!(y.strong_count(), 1);
            assert_eq!(y.weak_count(), 2);

            assert_eq!(**x.get(), 2);
            assert_eq!(**y.shared_non_reloading_get(), 1);

            // SAFETY:
            // No threading involved, &x and &y are valid.
            unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
            debug_println!("Dropping x");
            drop(x);
            debug_println!("Dropping y");
            drop(y);
        });
    }

    #[test]
    fn simple_item_state_enum_semantics() {
        dummy_model(|| {
            assert!(!ItemStateEnum::UndisturbedUndecorated.is_locked());
            assert!(ItemStateEnum::UndisturbedGcIsActive.is_locked());
            assert!(!ItemStateEnum::UndisturbedPayloadDropped.is_locked());
            assert!(ItemStateEnum::UndisturbedPayloadDroppedAndGcActive.is_locked());
            assert!(!ItemStateEnum::DisturbedUndecorated.is_locked());
            assert!(ItemStateEnum::DisturbedGcIsActive.is_locked());
            assert!(!ItemStateEnum::DisturbedPayloadDropped.is_locked());
            assert!(ItemStateEnum::DisturbedPayloadDroppedAndGcActive.is_locked());

            assert_eq!(
                ItemStateEnum::UndisturbedUndecorated.with_gc(),
                ItemStateEnum::UndisturbedGcIsActive
            );
            assert_eq!(
                ItemStateEnum::UndisturbedGcIsActive.with_gc(),
                ItemStateEnum::UndisturbedGcIsActive
            );
            assert_eq!(
                ItemStateEnum::UndisturbedPayloadDropped.with_gc(),
                ItemStateEnum::UndisturbedPayloadDroppedAndGcActive
            );
            assert_eq!(
                ItemStateEnum::UndisturbedPayloadDroppedAndGcActive.with_gc(),
                ItemStateEnum::UndisturbedPayloadDroppedAndGcActive
            );
            assert_eq!(
                ItemStateEnum::DisturbedUndecorated.with_gc(),
                ItemStateEnum::DisturbedGcIsActive
            );
            assert_eq!(
                ItemStateEnum::DisturbedGcIsActive.with_gc(),
                ItemStateEnum::DisturbedGcIsActive
            );
            assert_eq!(
                ItemStateEnum::DisturbedPayloadDropped.with_gc(),
                ItemStateEnum::DisturbedPayloadDroppedAndGcActive
            );
            assert_eq!(
                ItemStateEnum::DisturbedPayloadDroppedAndGcActive.with_gc(),
                ItemStateEnum::DisturbedPayloadDroppedAndGcActive
            );

            assert_eq!(
                ItemStateEnum::UndisturbedUndecorated.with_disturbed(),
                ItemStateEnum::DisturbedUndecorated
            );
            assert_eq!(
                ItemStateEnum::UndisturbedGcIsActive.with_disturbed(),
                ItemStateEnum::DisturbedGcIsActive
            );
            assert_eq!(
                ItemStateEnum::UndisturbedPayloadDropped.with_disturbed(),
                ItemStateEnum::DisturbedPayloadDropped
            );
            assert_eq!(
                ItemStateEnum::UndisturbedPayloadDroppedAndGcActive.with_disturbed(),
                ItemStateEnum::DisturbedPayloadDroppedAndGcActive
            );
            assert_eq!(
                ItemStateEnum::DisturbedUndecorated.with_disturbed(),
                ItemStateEnum::DisturbedUndecorated
            );
            assert_eq!(
                ItemStateEnum::DisturbedGcIsActive.with_disturbed(),
                ItemStateEnum::DisturbedGcIsActive
            );
            assert_eq!(
                ItemStateEnum::DisturbedPayloadDropped.with_disturbed(),
                ItemStateEnum::DisturbedPayloadDropped
            );
            assert_eq!(
                ItemStateEnum::DisturbedPayloadDroppedAndGcActive.with_disturbed(),
                ItemStateEnum::DisturbedPayloadDroppedAndGcActive
            );

            assert_eq!(
                ItemStateEnum::UndisturbedUndecorated.dropped(),
                ItemStateEnum::UndisturbedPayloadDropped
            );
            assert_eq!(
                ItemStateEnum::UndisturbedGcIsActive.dropped(),
                ItemStateEnum::UndisturbedPayloadDroppedAndGcActive
            );
            assert_eq!(
                ItemStateEnum::UndisturbedPayloadDropped.dropped(),
                ItemStateEnum::UndisturbedPayloadDropped
            );
            assert_eq!(
                ItemStateEnum::UndisturbedPayloadDroppedAndGcActive.dropped(),
                ItemStateEnum::UndisturbedPayloadDroppedAndGcActive
            );
            assert_eq!(
                ItemStateEnum::DisturbedUndecorated.dropped(),
                ItemStateEnum::DisturbedPayloadDropped
            );
            assert_eq!(
                ItemStateEnum::DisturbedGcIsActive.dropped(),
                ItemStateEnum::DisturbedPayloadDroppedAndGcActive
            );
            assert_eq!(
                ItemStateEnum::DisturbedPayloadDropped.dropped(),
                ItemStateEnum::DisturbedPayloadDropped
            );
            assert_eq!(
                ItemStateEnum::DisturbedPayloadDroppedAndGcActive.dropped(),
                ItemStateEnum::DisturbedPayloadDroppedAndGcActive
            );
        });
    }

    #[test]
    fn simple_create_clone_and_update_twice() {
        dummy_model(|| {
            let mut x = ArcShift::new(Box::new(45u32));
            let mut y = x.clone();
            assert_eq!(**x.get(), 45);
            x.update(Box::new(1u32));
            assert_eq!(**x.get(), 1);
            x.update(Box::new(21));
            assert_eq!(**x.get(), 21);
            // SAFETY:
            // No threading involved, &x and &y are valid.
            unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
            y.reload();
            assert_eq!(**y.get(), 21);
            // SAFETY:
            // No threading involved, &x and &y are valid.
            unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
        });
    }

    #[test]
    fn simple_create_clone_twice_and_update_twice() {
        dummy_model(|| {
            let mut x = ArcShift::new(Box::new(45u32));
            let mut y = x.clone();
            // SAFETY:
            // No threading involved, &x and &y are valid.
            unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
            assert_eq!(**x.get(), 45);
            x.update(Box::new(1u32));
            assert_eq!(x.weak_count(), 1);

            // SAFETY:
            // No threading involved, &x and &y are valid.
            unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
            let z = x.clone();
            assert_eq!(**x.get(), 1);
            x.update(Box::new(21));
            // SAFETY:
            // No threading involved, &x, &y and &z are valid.
            unsafe { ArcShift::debug_validate(&[&x, &y, &z], &[]) };
            assert_eq!(**x.get(), 21);
            y.reload();
            assert_eq!(**y.get(), 21);
            // SAFETY:
            // No threading involved, &x, &y and &z are valid.
            unsafe { ArcShift::debug_validate(&[&x, &y, &z], &[]) };
        });
    }

    #[test]
    #[cfg(all(feature = "std", not(any(loom, feature = "shuttle"))))]
    fn simple_threaded() {
        model(|| {
            let mut arc = ArcShift::new("Hello".to_string());
            let arc2 = arc.clone();

            let j1 = std::thread::Builder::new()
                .name("thread1".to_string())
                .spawn(move || {
                    //println!("Value in thread 1: '{}'", *arc); //Prints 'Hello'
                    arc.update("New value".to_string());
                    //println!("Updated value in thread 1: '{}'", *arc); //Prints 'New value'
                })
                .unwrap();

            let j2 = std::thread::Builder::new()
                .name("thread2".to_string())
                .spawn(move || {
                    // Prints either 'Hello' or 'New value', depending on scheduling:
                    let _a = arc2;
                    //println!("Value in thread 2: '{}'", arc2.get());
                })
                .unwrap();

            j1.join().unwrap();
            j2.join().unwrap();
        });
    }

    #[test]
    fn simple_upgrade_reg() {
        dummy_model(|| {
            let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let shift = ArcShift::new(crate::tests::leak_detection::InstanceSpy::new(
                count.clone(),
            ));
            // SAFETY:
            // No threading involved, &shift is valid.
            unsafe { ArcShift::debug_validate(&[&shift], &[]) };
            let shiftlight = ArcShift::downgrade(&shift);

            assert_eq!(shift.weak_count(), 2);

            // SAFETY:
            // No threading involved, &shift and &shiftlight is valid.
            unsafe { ArcShift::debug_validate(&[&shift], &[&shiftlight]) };

            debug_println!("==== running shift.get() = ");
            let mut shift2 = shiftlight.upgrade().unwrap();
            debug_println!("==== running arc.update() = ");
            // SAFETY:
            // No threading involved, &shift, 2shift2 and &shiftlight are valid.
            unsafe { ArcShift::debug_validate(&[&shift, &shift2], &[&shiftlight]) };
            shift2.update(crate::tests::leak_detection::InstanceSpy::new(
                count.clone(),
            ));

            // SAFETY:
            // No threading involved, &shift, &shift2 and &shiftlight are valid.
            unsafe { ArcShift::debug_validate(&[&shift, &shift2], &[&shiftlight]) };
        });
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
        t1.join().unwrap();
        t2.join().unwrap();
    });
}
#[test]
fn simple_threading2d() {
    model(|| {
        let owner = std::sync::Arc::new(SpyOwner2::new());
        {
            let mut shift = ArcShift::new(owner.create("orig"));
            let shiftlight = ArcShift::downgrade(&shift);
            // SAFETY:
            // No threading involved
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
            t1.join().unwrap();
            t2.join().unwrap();
        }
        owner.validate();
    });
}
#[test]
fn simple_threading_rcu_maybe() {
    model(|| {
        std::println!("== seed ==");
        let mut shift1 = ArcShift::new(0u32);
        let mut shift2 = ArcShift::clone(&shift1);
        let mut shift3 = ArcShift::clone(&shift1);
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                let mut temp = true;
                shift1.rcu_maybe(|prev| {
                    if temp {
                        temp = false;
                        Some(*prev + 1)
                    } else {
                        None
                    }
                });
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                let mut temp = true;
                shift2.rcu_maybe(|prev| {
                    if temp {
                        temp = false;
                        Some(*prev + 1)
                    } else {
                        None
                    }
                });
            })
            .unwrap();
        t1.join().unwrap();
        t2.join().unwrap();
        let x = *shift3.get();
        assert!(x >= 1);
        assert!(x <= 2);
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
        t1.join().unwrap();
        t2.join().unwrap();
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
        t1.join().unwrap();
        t2.join().unwrap();
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
        t1.join().unwrap();
        t2.join().unwrap();
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
        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();
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
        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();
    });
}
#[test]
#[cfg(not(loom))]
fn simple_threading3c() {
    model(|| {
        let shift1 = atomic::Arc::new(atomic::Mutex::new(ArcShift::new(42u32)));
        let shift2 = atomic::Arc::clone(&shift1);
        let mut shift3 = (shift1.lock().unwrap()).clone();
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
                shift3.update(45);
                debug_println!(" = drop t3 =");
            })
            .unwrap();
        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();
    });
}

#[cfg(not(loom))]
#[test]
fn simple_threading3d() {
    model(|| {
        let shift1 = atomic::Arc::new(atomic::Mutex::new(ArcShift::new(42u32)));
        let shift2 = atomic::Arc::clone(&shift1);
        let shift3 = shift1.clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                shift1.lock().unwrap().reload();
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
        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();
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

        t1.join().unwrap();
        t2.join().unwrap();
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
        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();
        assert_eq!(*shift0.get(), 6);
    });
}

#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn simple_threading4a() {
    model(|| {
        let shift1 = atomic::Arc::new(atomic::Mutex::new(ArcShift::new(42u32)));
        let shift2 = atomic::Arc::clone(&shift1);
        let shift3 = atomic::Arc::clone(&shift1);
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

        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();
        let ret = t4.join().unwrap();
        assert!(ret == 42 || ret == 43 || ret == 44);
    });
}

#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn simple_threading4b() {
    model(|| {
        let shift1 = atomic::Arc::new(ArcShift::new(42u32));
        let shift2 = atomic::Arc::clone(&shift1);
        let shift3 = atomic::Arc::clone(&shift1);
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
                *t
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

        t1.join().unwrap();
        t2.join().unwrap();
        let ret3 = t3.join().unwrap();
        assert!(ret3 == 44 || ret3 == 43);
        let ret = t4.join().unwrap();
        assert!(ret.is_none() || ret == Some(43) || ret == Some(44) || ret == Some(42));
    });
}
#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn simple_threading4c() {
    model(|| {
        let count = atomic::Arc::new(SpyOwner2::new());
        {
            let shift1 = atomic::Arc::new(atomic::Mutex::new(ArcShift::new(count.create("orig"))));
            let shift2 = atomic::Arc::clone(&shift1);
            let shift3 = atomic::Arc::clone(&shift1);
            let shift4 = atomic::Arc::clone(&shift1);
            let _t = atomic::Arc::clone(&shift1);
            let count1 = count.clone();
            let count2 = count.clone();
            let t1 = atomic::thread::Builder::new()
                .name("t1".to_string())
                .stack_size(100_000)
                .spawn(move || {
                    debug_println!(" = On thread t1 = {:?}", std::thread::current().id());
                    shift1.lock().unwrap().update(count1.create("t1val"));
                    debug_println!(" = drop t1 =");
                })
                .unwrap();

            let t2 = atomic::thread::Builder::new()
                .name("t2a".to_string())
                .stack_size(100_000)
                .spawn(move || {
                    debug_println!(" = On thread t2 = {:?}", std::thread::current().id());
                    let mut _shift = shift2; //.clone()

                    debug_println!(" = drop t2 =");
                })
                .unwrap();

            let t3 = atomic::thread::Builder::new()
                .name("t3".to_string())
                .stack_size(100_000)
                .spawn(move || {
                    debug_println!(" = On thread t3 = {:?}", std::thread::current().id());
                    shift3.lock().unwrap().update(count2.create("t3val"));

                    debug_println!(" = drop t3 =");
                })
                .unwrap();
            let t4 = atomic::thread::Builder::new()
                .name("t4".to_string())
                .stack_size(100_000)
                .spawn(move || {
                    debug_println!(" = On thread t4 = {:?}", std::thread::current().id());
                    let shift4 = &*shift4;
                    //verify_item(shift4.item.as_ptr());
                    let _t = std::hint::black_box(shift4);
                    debug_println!(" = drop t4 =");
                })
                .unwrap();

            t1.join().unwrap();
            t2.join().unwrap();
            t3.join().unwrap();
            t4.join().unwrap();
        }
        debug_println!("All threads stopped");
        count.validate();
    });
}

#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn simple_threading4d() {
    model(|| {
        let count = atomic::Arc::new(SpyOwner2::new());
        {
            let count3 = count.clone();
            let shift1 = atomic::Arc::new(ArcShift::new(count.create("orig")));

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
                    let _t = std::hint::black_box(shift3.shared_non_reloading_get());
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

            t1.join().unwrap();
            t2.join().unwrap();
            t3.join().unwrap();
            t4.join().unwrap();
            debug_println!("Joined all threads");
        }

        count.validate();
        drop(count);
    });
}

#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn simple_threading4e() {
    model(|| {
        let shift = atomic::Arc::new(atomic::Mutex::new(ArcShift::new(42u32)));
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

        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();
        t4.join().unwrap();
    });
}

#[cfg(not(any(loom, feature = "shuttle")))]
#[test]
fn simple_test_clones2() {
    let shift = ArcShift::new("orig".to_string());
    let shift1 = ArcShift::downgrade(&shift);
    let shift2 = shift.clone();
    let shift3 = shift.clone();
    // SAFETY:
    // No threading involved
    unsafe { ArcShift::debug_validate(&[&shift, &shift2, &shift3], &[&shift1]) };
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
        t1.join().unwrap();
        t2.join().unwrap();
    });
}
#[test]
fn simple_threading_repro1() {
    model(|| {
        debug_println!("-------- loom -------------");
        let root = ArcShift::new(42u32);
        let mut curval = root.clone();
        let light = ArcShift::downgrade(&curval);
        // SAFETY:
        // No threading involved
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
        t1.join().unwrap();
        t2.join().unwrap();

        // SAFETY:
        // No threading involved
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
        // SAFETY:
        // No threading involved
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
        t1.join().unwrap();
        t2.join().unwrap();
    });
}

#[cfg(not(any(loom, feature = "shuttle")))]
#[test]
fn simple_threading_repro2() {
    debug_println!("-------- loom -------------");
    let root = ArcShift::new(42u32);
    let mut curval = root.clone();
    let light = ArcShift::downgrade(&curval);
    curval.update(42);
    debug_println!("----> curval.dropping");
    drop(curval);

    std::println!("----> light.upgrade");
    light.upgrade();
    // SAFETY:
    // No threading involved
    unsafe { ArcShift::debug_validate(&[&root], &[&light]) };
    drop(light);
}

#[test]
fn simple_threading_update_twice() {
    model(|| {
        debug_println!("-------- loom -------------");

        let owner = alloc::sync::Arc::new(SpyOwner2::new());
        {
            let owner1 = owner.clone();
            let owner2 = owner.clone();

            let mut shift = ArcShift::new(owner.create("1"));
            let mut shift1 = shift.clone();
            let shift2 = shift.clone();
            // SAFETY:
            // No threading involved
            unsafe { ArcShift::debug_validate(&[&shift, &shift1, &shift2], &[]) };
            let t1 = atomic::thread::Builder::new()
                .name("t1".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    shift.update(owner1.create("t1"));
                    debug_println!("--> t1 dropping");
                })
                .unwrap();

            let t2 = atomic::thread::Builder::new()
                .name("t2".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    shift1.update(owner2.create("t2"));
                    debug_println!("--> t2 dropping");
                })
                .unwrap();
            t1.join().unwrap();
            t2.join().unwrap();
            // SAFETY:
            // No threading involved
            unsafe { ArcShift::debug_validate(&[&shift2], &[]) };
            debug_println!("--> Main dropping");
        }
        owner.validate();
    });
}

// Building a recursive data structure using ArcShift is possible, but unfortunately not
// super useful. It is not possible to actually modify the contents of an ArcShift. This means,
// if those contents include an ArcShift instance, that instance cannot be reloaded, unless
// interior mutability is in play. And such interior mutability would have to be a mutex
// in the multi-threaded case, calling the design into question (since ArcShift is meant to
// be a faster alternative to mutexes). In the non-multithreaded case you could use
// ArcShiftCell, but in that case, why even use ArcShift at all, why not just use Rc<RefCell<T>>?
//
// Anyway, this test case verifies that it at least does work, with no leaks.
#[test]
fn test_recursive_structure() {
    dummy_model(|| {
        use std::string::String;
        #[derive(Clone)]
        struct Node {
            parent: Option<ArcShiftWeak<Node>>,
            children: std::vec::Vec<ArcShiftCell<Node>>,
            value: String,
        }

        let mut root = ArcShift::new(Node {
            parent: None,
            children: vec![],
            value: "root".into(),
        });

        let child1 = ArcShift::new(Node {
            parent: Some(ArcShift::downgrade(&root)),
            children: vec![],
            value: "child1".into(),
        });
        let mut child2 = ArcShift::new(Node {
            parent: Some(ArcShift::downgrade(&root)),
            children: vec![],
            value: "child2".into(),
        });

        root.rcu(|prev| {
            let mut new = prev.clone();
            new.children
                .push(ArcShiftCell::from_arcshift(child1.clone()));
            new.children
                .push(ArcShiftCell::from_arcshift(child2.clone()));
            new
        });

        let mut child21 = ArcShift::new(Node {
            parent: Some(ArcShift::downgrade(&child2)),
            children: vec![],
            value: "child21".into(),
        });

        child2.rcu(|prev| {
            let mut new = prev.clone();
            new.children
                .push(ArcShiftCell::from_arcshift(child21.clone()));
            new
        });
        drop(child2);
        drop(child1);

        assert_eq!(
            root.get().children[1].borrow().children[0].borrow().value,
            "child21"
        );

        child21.rcu(|prev| {
            let mut new = prev.clone();
            new.value = "Banana".to_string();
            new
        });
        assert_eq!(
            root.get().children[1].borrow().children[0].borrow().value,
            "Banana"
        );
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
        // SAFETY:
        // No threading involved
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
        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift3], &[]) };
        debug_println!("--> Main dropping");
        assert!(*shift3.get() > 42);
    });
}

#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn simple_threading_update_four() {
    model(|| {
        debug_println!("-------- loom -------------");
        let mut shift = ArcShift::new(42u32);
        let mut shift1 = shift.clone();
        let mut shift2 = shift.clone();
        let mut shift3 = shift.clone();
        let mut shift4 = shift.clone();
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift, &shift1, &shift2, &shift3, &shift4], &[]) };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift1.update(43);
                debug_println!("--> t1 dropping");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift2.update(44);
                debug_println!("--> t2 dropping");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift3.update(45);
                debug_println!("--> t3 dropping");
            })
            .unwrap();

        let t4 = atomic::thread::Builder::new()
            .name("t4".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift4.update(46);
                debug_println!("--> t4 dropping");
            })
            .unwrap();
        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();
        t4.join().unwrap();
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift], &[]) };
        debug_println!("--> Main dropping");
        assert!(*shift.get() >= 43 && *shift.get() <= 46);
    });
}

#[test]
fn simple_threading_drop_four() {
    model(|| {
        debug_println!("-------- loom -------------");
        let shift = ArcShift::new(42u32);
        let mut shift1 = shift.clone();
        shift1.update(43);
        let mut shift2 = shift1.clone();
        shift2.update(44);
        let mut shift3 = shift2.clone();
        shift3.update(45);
        let mut shift4 = shift3.clone();
        shift4.update(46);
        drop(shift);
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift1, &shift2, &shift3, &shift4], &[]) };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift1.update(42);
                let _ = shift1;
                debug_println!("--> t1 dropping");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                let _ = shift2;
                debug_println!("--> t2 dropping");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                let _ = shift3;
                debug_println!("--> t3 dropping");
            })
            .unwrap();

        let t4 = atomic::thread::Builder::new()
            .name("t4".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                let _ = shift4;
                debug_println!("--> t4 dropping");
            })
            .unwrap();
        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();
        t4.join().unwrap();
    });
}

#[test]
fn simple_threading_try_get_mut3() {
    model(|| {
        debug_println!("-------- loom -------------");
        let mut shift = ArcShift::new(1);
        let mut shift1 = shift.clone();
        let mut shift2 = shift.clone();
        let mut shift3 = shift.clone();
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift, &shift1, &shift2, &shift3], &[]) };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                if let Some(x) = shift.try_get_mut() {
                    assert!(*x == 1 || *x == 2);
                    *x = 41;
                }
                debug_println!("--> t1 dropping");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                if let Some(x) = shift1.try_get_mut() {
                    assert!(*x == 1 || *x == 2);
                    *x = 42;
                }
                debug_println!("--> t2 dropping");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift2.update(2);
                debug_println!("--> t3 dropping");
            })
            .unwrap();
        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift3], &[]) };
        debug_println!("--> Main dropping");
        assert!(*shift3.get() > 1);
    });
}

#[test]
fn simple_threading_try_get_mut2() {
    let saw_nonzero = alloc::sync::Arc::new(core::sync::atomic::AtomicBool::new(false));
    #[allow(unused)]
    let saw_nonzero2 = saw_nonzero.clone();
    model(move || {
        debug_println!("-------- loom -------------");
        let shift = ArcShift::new(0);
        let mut shift1 = shift.clone();
        let mut shift2 = shift.clone();
        drop(shift);
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift1, &shift2], &[]) };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                if let Some(x) = shift1.try_get_mut() {
                    assert_eq!(*x, 0);
                    *x = 2;
                }
                debug_println!("--> t1 dropping");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                if let Some(x) = shift2.try_get_mut() {
                    assert_eq!(*x, 0);
                    *x = 3;
                }
                debug_println!("--> t2 dropping");
                shift2
            })
            .unwrap();
        t1.join().unwrap();
        let mut shift = t2.join().unwrap();
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift], &[]) };
        debug_println!("--> Main dropping");
        if *shift.get() > 0 {
            saw_nonzero.store(true, Ordering::Relaxed);
        }
        assert!(*shift.get() < 4);
    });

    #[cfg(loom)]
    assert!(saw_nonzero2.load(Ordering::Relaxed));
}

#[test]
fn simple_threading_shared_get_update() {
    let seen_values = alloc::sync::Arc::new(core::sync::atomic::AtomicU8::new(0));
    let _seen_values2 = seen_values.clone();
    model(move || {
        debug_println!("-------- loom -------------");
        let shift = ArcShift::new(0u32);
        let shift1 = shift.clone();
        let mut shift2 = shift.clone();
        drop(shift);
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift1, &shift2], &[]) };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || *shift1.shared_get())
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift2.update(1);
            })
            .unwrap();
        let r1 = t1.join().unwrap();
        t2.join().unwrap();
        debug_println!("--> Main dropping");

        assert!(r1 == 0 || r1 == 1);
        seen_values.fetch_or(1 << r1, Ordering::Relaxed);
    });
    #[cfg(loom)]
    {
        assert_eq!(_seen_values2.load(Ordering::Relaxed), 3);
    }
}
#[test]
fn simple_threading_shared_get_twice_update() {
    let seen_values = alloc::sync::Arc::new(core::sync::atomic::AtomicU8::new(0));
    let _seen_values2 = seen_values.clone();
    model(move || {
        debug_println!("-------- loom -------------");
        let shift = ArcShift::new(0u32);
        let shift1 = shift.clone();
        let mut shift2 = shift.clone();
        drop(shift);
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift1, &shift2], &[]) };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || *shift1.shared_get())
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift2.update(1);
                shift2.update(2);
            })
            .unwrap();
        let r1 = t1.join().unwrap();
        t2.join().unwrap();
        debug_println!("--> Main dropping");

        assert!(r1 == 0 || r1 == 1 || r1 == 2);
        seen_values.fetch_or(1 << r1, Ordering::Relaxed);
    });
    #[cfg(loom)]
    {
        assert_eq!(_seen_values2.load(Ordering::Relaxed), 7);
    }
}

#[test]
fn simple_threading_shared_get_drop() {
    model(move || {
        debug_println!("-------- loom -------------");
        let shift = ArcShift::new(0u32);
        let shift1 = shift.clone();
        let shift2 = shift.clone();
        drop(shift);
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift1, &shift2], &[]) };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || *shift1.shared_get())
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                drop(shift2);
            })
            .unwrap();
        let r1 = t1.join().unwrap();
        t2.join().unwrap();
        debug_println!("--> Main dropping");

        assert!(r1 == 0);
    });
}

#[test]
fn simple_threading_try_into_inner2() {
    model(move || {
        debug_println!("-------- loom -------------");
        let shift = ArcShift::new(0);
        let shift1 = shift.clone();
        let shift2 = shift.clone();
        drop(shift);
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift1, &shift2], &[]) };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                if let Some(x) = shift1.try_into_inner() {
                    assert_eq!(x, 0);
                    return true;
                }
                debug_println!("--> t1 dropping");
                false
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                if let Some(x) = shift2.try_into_inner() {
                    assert_eq!(x, 0);
                    return true;
                }
                debug_println!("--> t2 dropping");
                false
            })
            .unwrap();
        let r1 = t1.join().unwrap();
        let r2 = t2.join().unwrap();
        debug_println!("--> Main dropping");

        assert_ne!(r1, r2);
    });
}
#[test]
fn simple_threading_try_into_inner3() {
    model(move || {
        debug_println!("-------- loom -------------");
        let shift = ArcShift::new(0);
        let shift1 = shift.clone();
        let shift2 = shift.clone();
        let shift3 = shift.clone();
        drop(shift);
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift1, &shift2, &shift3], &[]) };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                if let Some(x) = shift1.try_into_inner() {
                    assert_eq!(x, 0);
                    return true;
                }
                debug_println!("--> t1 dropping");
                false
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                if let Some(x) = shift2.try_into_inner() {
                    assert_eq!(x, 0);
                    return true;
                }
                debug_println!("--> t2 dropping");
                false
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                if let Some(x) = shift3.try_into_inner() {
                    assert_eq!(x, 0);
                    return true;
                }
                debug_println!("--> t3 dropping");
                false
            })
            .unwrap();

        let r1 = t1.join().unwrap();
        let r2 = t2.join().unwrap();
        let r3 = t3.join().unwrap();
        debug_println!("--> Main dropping");
        assert_eq!(r1 as u32 + r2 as u32 + r3 as u32, 1);
    });
}

/*
This does not compile, nor should it compile.
ArcShift<T:'a> must be invariant in 'a.
I.e, ArcShift<T:'a> and ArcShift<T:'b> are not compatible unless 'a == 'b .
#[allow(warnings)]
#[test]
fn variance_test() {
    fn inner() -> ArcShift<&'static str>{
        let s = "Hello".to_string();
        struct Example<'a> {
            a: ArcShift<&'a str>
        }
        let global: Example<'static> = Example {
            a: ArcShift::new("static")
        };
        let global_clone = global.a.clone();

        let local: Example = Example {
            a: ArcShift::new(s.as_str())
        };

        fn test<'a>(x: Example<'a>, y: Example<'a>) {
            let Example{a:x} = x;
            let Example{a:mut y} = y;
            let short_lived = x.try_into_inner().unwrap();
            y.update(short_lived);
        }

        test(local, global);
        global_clone
    }

    let mut ohoh = inner();
    std::println!("{}", ohoh.get());

}
*/

#[test]
fn simple_threading_update_downgrade_shared_get() {
    model(|| {
        /*
        0: DropLight(0) DowngradeLight(0), ReadArc { arc: 0 } CloneArc { from: 0, to: 2 } DropArc(0), UpgradeLight(0)
        1:  CloneArc { from: 1, to: 2 }, CloneArcLight { from: 1, to: 0 },  UpgradeLight(1), CloneArcLight { from: 1, to: 2 } DropLight(1), CreateUpdateArc(1
        2: ReadArc { arc: 2 } DropArc(2) ReadArc { arc: 2 } CreateUpdateArc(1, InstanceSpy2 , name: "2" }) CloneArc { from: 2, to: 1 }, CloneArcLight { from: 2, to: 0 }, UpgradeLight(2) SharedReadArc { arc: 2 }
        */

        debug_println!("-------- loom -------------");
        let mut shift = ArcShift::new(42u32);
        let shift0 = shift.clone();
        let mut shift1 = shift.clone();
        let mut shift2 = shift.clone();
        let weakshift0 = ArcShift::downgrade(&shift);
        let weakshift1 = ArcShift::downgrade(&shift);
        let weakshift2 = ArcShift::downgrade(&shift);
        // SAFETY:
        // No threading involved
        unsafe {
            ArcShift::debug_validate(
                &[&shift, &shift0, &shift1, &shift2],
                &[&weakshift0, &weakshift1, &weakshift2],
            )
        };
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                drop(weakshift0);
                let _t = shift0.clone();
                drop(shift0);
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                let _t = shift1.clone();
                let lt = weakshift1.clone();
                let _new = lt.upgrade();
                let _lt2 = lt.clone();
                drop(weakshift1);
                shift1.update(43);
                debug_println!("--> t2 dropping");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                black_box(shift2.get());
                drop(shift2);
                let new = weakshift2.upgrade();
                if let Some(new) = new {
                    new.shared_non_reloading_get();
                }
                debug_println!("--> t3 dropping");
            })
            .unwrap();
        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();
        // SAFETY:
        // No threading involved
        unsafe { ArcShift::debug_validate(&[&shift], &[]) };
        debug_println!("--> Main dropping");
        assert_eq!(*shift.get(), 43);
    });
}
