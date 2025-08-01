//! Fuzzing-test cases which explicit focus on simultaneous operations in different threads

use crate::tests::leak_detection::SpyOwner2;
use crate::tests::{model, model2, InstanceSpy2};
use crate::{atomic, ArcShift, ArcShiftWeak};
use std::println;
use std::string::ToString;
use std::vec;
use std::vec::Vec;

fn generic_3thread_ops_a<
    F1: Fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
    F2: Fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
    F3: Fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
>(
    f1: F1,
    f2: F2,
    f3: F3,
) {
    let f1 = std::sync::Arc::new(f1);
    let f2 = std::sync::Arc::new(f2);
    let f3 = std::sync::Arc::new(f3);
    model(move || {
        debug_println!("---- seed ----");
        let f1 = f1.clone();
        let f2 = f2.clone();
        let f3 = f3.clone();
        let owner = std::sync::Arc::new(SpyOwner2::new());
        {
            let shift1 = ArcShift::new(owner.create("orig"));
            let shift2 = shift1.clone();
            let shift3 = shift1.clone();
            let owner_ref1 = owner.clone();
            let owner_ref2 = owner.clone();
            let owner_ref3 = owner.clone();

            let t1 = atomic::thread::Builder::new()
                .name("t1".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    debug_println!(" = On thread t1 =");
                    f1(&owner_ref1, shift1, "t1")
                })
                .unwrap();

            let t2 = atomic::thread::Builder::new()
                .name("t2".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    debug_println!(" = On thread t2 =");
                    f2(&owner_ref2, shift2, "t2")
                })
                .unwrap();

            let t3 = atomic::thread::Builder::new()
                .name("t3".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    debug_println!(" = On thread t3 =");
                    f3(&owner_ref3, shift3, "t3")
                })
                .unwrap();
            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            _ = t3.join().unwrap();
        }
        owner.validate();
    });
}

fn generic_3thread_ops_b<
    F1: Fn(
            &SpyOwner2,
            ArcShiftWeak<InstanceSpy2>,
            &'static str,
        ) -> Option<ArcShiftWeak<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
    F2: Fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
    F3: Fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
>(
    f1: F1,
    f2: F2,
    f3: F3,
    repro: Option<&str>,
) {
    let f1 = std::sync::Arc::new(f1);
    let f2 = std::sync::Arc::new(f2);
    let f3 = std::sync::Arc::new(f3);
    model2(
        move || {
            let f1 = f1.clone();
            let f2 = f2.clone();
            let f3 = f3.clone();
            let owner = std::sync::Arc::new(SpyOwner2::new());
            {
                let mut shift = ArcShift::new(owner.create("orig1"));
                let shift1 = ArcShift::downgrade(&shift);
                shift.update(owner.create("orig2"));
                let shift2 = shift.clone();
                shift.update(owner.create("orig3"));
                let shift3 = shift.clone();

                debug_println!("Prior to debug_validate");
                // SAFETY:
                // No threading involved
                unsafe { ArcShift::debug_validate(&[&shift, &shift2, &shift3], &[&shift1]) };
                debug_println!("Post debug_validate");

                let owner_ref1 = owner.clone();
                let owner_ref2 = owner.clone();
                let owner_ref3 = owner.clone();

                let t1 = atomic::thread::Builder::new()
                    .name("t1".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t1 =");
                        let t = f1(&owner_ref1, shift1, "t1");
                        debug_println!(" = thread 1 dropping =");
                        t
                    })
                    .unwrap();

                let t2 = atomic::thread::Builder::new()
                    .name("t2".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t2 =");
                        let t = f2(&owner_ref2, shift2, "t2");

                        debug_println!(" = thread 2 dropping =");
                        t
                    })
                    .unwrap();

                let t3 = atomic::thread::Builder::new()
                    .name("t3".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t3 =");
                        let t = f3(&owner_ref3, shift3, "t3");
                        debug_println!(" = thread 3 dropping =");
                        t
                    })
                    .unwrap();
                debug_println!("Begin joining threads");
                _ = t1.join().unwrap();
                _ = t2.join().unwrap();
                _ = t3.join().unwrap();
                // SAFETY:
                // No threading involved
                unsafe { ArcShift::debug_validate(&[&shift], &[]) };
                debug_println!("Joined all threads");
            }
            owner.validate();
        },
        repro,
    );
}

fn generic_2thread_ops_b<
    F1: Fn(
            &SpyOwner2,
            ArcShiftWeak<InstanceSpy2>,
            &'static str,
        ) -> Option<ArcShiftWeak<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
    F2: Fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
>(
    f1: F1,
    f2: F2,
    repro: Option<&str>,
) {
    let f1 = std::sync::Arc::new(f1);
    let f2 = std::sync::Arc::new(f2);
    model2(
        move || {
            debug_println!("-- Seed --");
            let f1 = f1.clone();
            let f2 = f2.clone();
            let owner = std::sync::Arc::new(SpyOwner2::new());
            {
                let shift = ArcShift::new(owner.create("orig"));
                let shift1 = ArcShift::downgrade(&shift);
                let shift2 = shift.clone();

                debug_println!("Prior to debug_validate");
                // SAFETY:
                // No threading involved
                unsafe { ArcShift::debug_validate(&[&shift, &shift2], &[&shift1]) };
                debug_println!("Post debug_validate");

                let owner_ref1 = owner.clone();
                let owner_ref2 = owner.clone();

                let t1 = atomic::thread::Builder::new()
                    .name("t1".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t1 =");
                        let t = f1(&owner_ref1, shift1, "t1");
                        debug_println!(" = thread 1 dropping =");
                        t
                    })
                    .unwrap();

                let t2 = atomic::thread::Builder::new()
                    .name("t2".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t2 =");
                        let t = f2(&owner_ref2, shift2, "t2");

                        debug_println!(" = thread 2 dropping =");
                        t
                    })
                    .unwrap();

                debug_println!("Begin joining threads");
                _ = t1.join().unwrap();
                _ = t2.join().unwrap();
                // SAFETY:
                // No threading involved
                unsafe { ArcShift::debug_validate(&[&shift], &[]) };
                debug_println!("Joined all threads");
            }
            owner.validate();
        },
        repro,
    );
}

// Hackpology: There's some code duplication and DRY-violations here
// we should clean this up at some point.
fn generic_3thread_ops_c<
    F1: Fn(
            &SpyOwner2,
            ArcShiftWeak<InstanceSpy2>,
            &'static str,
        ) -> Option<ArcShiftWeak<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
    F2: Fn(
            &SpyOwner2,
            ArcShiftWeak<InstanceSpy2>,
            &'static str,
        ) -> Option<ArcShiftWeak<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
    F3: Fn(
            &SpyOwner2,
            ArcShiftWeak<InstanceSpy2>,
            &'static str,
        ) -> Option<ArcShiftWeak<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
>(
    f1: F1,
    f2: F2,
    f3: F3,
    repro: Option<&str>,
) {
    let f1 = std::sync::Arc::new(f1);
    let f2 = std::sync::Arc::new(f2);
    let f3 = std::sync::Arc::new(f3);
    model2(
        move || {
            let f1 = f1.clone();
            let f2 = f2.clone();
            let f3 = f3.clone();
            let owner = std::sync::Arc::new(SpyOwner2::new());
            {
                let mut shift = ArcShift::new(owner.create("orig1"));
                let shift1 = ArcShift::downgrade(&shift);
                shift.update(owner.create("orig2"));
                let shift2 = ArcShift::downgrade(&shift);
                shift.update(owner.create("orig3"));
                let shift3 = ArcShift::downgrade(&shift);
                debug_println!("Prior to debug_validate");
                // SAFETY:
                // No threading involved
                unsafe { ArcShift::debug_validate(&[&shift], &[&shift1, &shift2, &shift3]) };
                debug_println!("Post debug_validate");
                drop(shift);
                // SAFETY:
                // No threading involved
                unsafe { ArcShift::debug_validate(&[], &[&shift1, &shift2, &shift3]) };

                let owner_ref1 = owner.clone();
                let owner_ref2 = owner.clone();
                let owner_ref3 = owner.clone();

                let t1 = atomic::thread::Builder::new()
                    .name("t1".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t1 =");
                        let t = f1(&owner_ref1, shift1, "t1");
                        debug_println!(" = thread 1 dropping =");
                        t
                    })
                    .unwrap();

                let t2 = atomic::thread::Builder::new()
                    .name("t2".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t2 =");
                        let t = f2(&owner_ref2, shift2, "t2");

                        debug_println!(" = thread 2 dropping =");
                        t
                    })
                    .unwrap();

                let t3 = atomic::thread::Builder::new()
                    .name("t3".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t3 =");
                        let t = f3(&owner_ref3, shift3, "t3");
                        debug_println!(" = thread 3 dropping =");
                        t
                    })
                    .unwrap();
                debug_println!("Begin joining threads");
                _ = t1.join().unwrap();
                _ = t2.join().unwrap();
                _ = t3.join().unwrap();
                debug_println!("Joined all threads");
            }
            owner.validate();
        },
        repro,
    );
}

fn generic_2thread_ops_d<
    F1: Fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
    F2: Fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
>(
    f1: F1,
    f2: F2,
    repro: Option<&str>,
) {
    let f1 = std::sync::Arc::new(f1);
    let f2 = std::sync::Arc::new(f2);
    model2(
        move || {
            debug_println!("-- Seed --");
            let f1 = f1.clone();
            let f2 = f2.clone();
            let owner = std::sync::Arc::new(SpyOwner2::new());
            {
                let mut shift1 = ArcShift::new(owner.create("orig"));
                let shift2 = shift1.clone();
                shift1.update(owner.create("B"));
                shift1.update(owner.create("C"));
                shift1.update(owner.create("D"));

                let owner_ref1 = owner.clone();
                let owner_ref2 = owner.clone();

                let t1 = atomic::thread::Builder::new()
                    .name("t1".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t1 =");
                        let t = f1(&owner_ref1, shift1, "t1");
                        debug_println!(" = thread 1 dropping =");
                        t
                    })
                    .unwrap();

                let t2 = atomic::thread::Builder::new()
                    .name("t2".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t2 =");
                        let t = f2(&owner_ref2, shift2, "t2");
                        debug_println!(" = thread 2 dropping =");
                        t
                    })
                    .unwrap();

                debug_println!("Begin joining threads");
                _ = t1.join().unwrap();
                _ = t2.join().unwrap();
                debug_println!("Joined all threads");
            }
            owner.validate();
        },
        repro,
    );
}

fn generic_2thread_ops_e<
    F1: Fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
    F2: Fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>
        + Sync
        + Send
        + 'static,
>(
    f1: F1,
    f2: F2,
    repro: Option<&str>,
) {
    let f1 = std::sync::Arc::new(f1);
    let f2 = std::sync::Arc::new(f2);
    model2(
        move || {
            debug_println!("-- Seed --");
            let f1 = f1.clone();
            let f2 = f2.clone();
            let owner = std::sync::Arc::new(SpyOwner2::new());
            {
                let mut shift1 = ArcShift::new(owner.create("orig"));
                let shift2 = shift1.clone();
                shift1.update(owner.create("B"));
                shift1.update(owner.create("C"));
                shift1.update(owner.create("D"));
                let mut shift3 = shift1.clone();
                shift3.update(owner.create("E"));
                shift3.update(owner.create("F"));
                drop(shift3);

                let owner_ref1 = owner.clone();
                let owner_ref2 = owner.clone();

                let t1 = atomic::thread::Builder::new()
                    .name("t1".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t1 =");
                        let t = f1(&owner_ref1, shift1, "t1");
                        debug_println!(" = thread 1 dropping =");
                        t
                    })
                    .unwrap();

                let t2 = atomic::thread::Builder::new()
                    .name("t2".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t2 =");
                        let t = f2(&owner_ref2, shift2, "t2");
                        debug_println!(" = thread 2 dropping =");
                        t
                    })
                    .unwrap();

                debug_println!("Begin joining threads");
                _ = t1.join().unwrap();
                _ = t2.join().unwrap();
                debug_println!("Joined all threads");
            }
            owner.validate();
        },
        repro,
    );
}

#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn generic_3threading_b_000() {
    generic_3threading_b_all_impl(0, 0, 0, None);
}

/* This is covered by the test above.
#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn generic_3threading_b_300() {
    generic_3threading_b_all_impl(3, 0, 0, None);
}
*/

fn generic_3threading_b_all_impl(skip1: usize, skip2: usize, skip3: usize, repro: Option<&str>) {
    let limit = if skip1 != 0 || skip2 != 0 || skip3 != 0 {
        1
    } else {
        usize::MAX / 10
    };
    #[allow(clippy::type_complexity)]
    let ops1: Vec<
        fn(
            &SpyOwner2,
            ArcShiftWeak<InstanceSpy2>,
            &'static str,
        ) -> Option<ArcShiftWeak<InstanceSpy2>>,
    > = vec![
        |_, shift, _| {
            debug_println!("====> shift.upgrade()");
            _ = shift.upgrade();
            Some(shift)
        },
        |_, shift, _| {
            debug_println!("====> shift.clone()");
            _ = shift.clone();
            Some(shift)
        },
        |_, shift, _| Some(shift),
        |owner, shift, thread| {
            debug_println!("====> shift.upgrade()");
            let upgraded = shift.upgrade();
            if let Some(mut upgraded) = upgraded {
                debug_println!("====> upgraded.upgrade()");
                upgraded.update(owner.create(thread))
            }
            Some(shift)
        },
        |_, _, _| None,
    ];
    #[allow(clippy::type_complexity)]
    let ops23: Vec<
        fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>,
    > = vec![
        |owner, mut shift, thread| {
            debug_println!("====> shift.update()");
            shift.update(owner.create(thread));
            Some(shift)
        },
        |_owner, mut shift, _thread| {
            debug_println!("====> shift.get()");
            std::hint::black_box(shift.get());
            Some(shift)
        },
        |_owner, shift, _thread| {
            debug_println!("====> shift.shared_get()");
            std::hint::black_box(shift.shared_non_reloading_get());
            Some(shift)
        },
        |_owner, mut shift, _thread| {
            debug_println!("====> shift.reload()");
            shift.reload();
            Some(shift)
        },
        |_owner, _shift, _thread| None,
    ];
    for (_n1, op1) in ops1.iter().enumerate().skip(skip1).take(limit) {
        for (_n2, op2) in ops23.iter().enumerate().skip(skip2).take(limit) {
            for (_n3, op3) in ops23.iter().enumerate().skip(skip3).take(limit) {
                {
                    println!("\n");
                    println!(
                        " ===================== {_n1} {_n2} {_n3} ======================"
                    );
                    println!("\n");
                }
                generic_3thread_ops_b(*op1, *op2, *op3, repro)
            }
        }
    }
}
#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn generic_3threading_a_all() {
    generic_3threading_a_all_impl(0, 0, 0)
}

#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn generic_3threading_a_3() {
    generic_3threading_a_all_impl(3, 0, 0)
}
#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn generic_3threading_a_5() {
    generic_3threading_a_all_impl(5, 0, 0)
}

#[test]
#[cfg(not(feature = "disable_slow_tests"))]
fn generic_3threading_a_025() {
    generic_3threading_a_all_impl(0, 2, 5)
}

#[cfg(not(feature = "disable_slow_tests"))]
fn generic_3threading_a_all_impl(skip0: usize, skip1: usize, skip2: usize) {
    #[allow(clippy::type_complexity)]
    let ops: Vec<
        fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>,
    > = vec![
        |owner, mut shift, thread| {
            debug_println!("====> shift.update()");
            shift.update(owner.create(thread));
            Some(shift)
        },
        |owner, mut shift, thread| {
            debug_println!("====> shift.update()");
            shift.update(owner.create(thread));
            Some(shift)
        },
        |_owner, mut shift, _thread| {
            debug_println!("====> shift.get()");
            std::hint::black_box(shift.get());
            Some(shift)
        },
        |_owner, shift, _thread| {
            debug_println!("====> shift.shared_get()");
            std::hint::black_box(shift.shared_non_reloading_get());
            Some(shift)
        },
        |_owner, mut shift, _thread| {
            debug_println!("====> shift.reload()");
            shift.reload();
            Some(shift)
        },
        |_owner, _shift, _thread| {
            debug_println!("====> drop()");
            None
        },
    ];
    for (n1, op1) in ops.iter().enumerate().skip(skip0) {
        for (n2, op2) in ops.iter().enumerate().skip(skip1) {
            for (n3, op3) in ops.iter().enumerate().skip(skip2) {
                {
                    println!("========= {n1} {n2} {n3} ==========");
                }
                generic_3thread_ops_a(*op1, *op2, *op3);
                if skip0 != 0 || skip1 != 0 || skip2 != 0 {
                    return;
                }
            }
        }
    }
}

#[test]
fn generic_3threading1() {
    generic_3thread_ops_a(
        |owner1, mut shift1, thread| {
            shift1.update(owner1.create(thread));
            debug_println!("thread1 update done");
            Some(shift1)
        },
        |owner2, mut shift2, thread| {
            shift2.update(owner2.create(thread));
            debug_println!("thread2 update done");
            Some(shift2)
        },
        |owner3, mut shift3, thread| {
            shift3.update(owner3.create(thread));
            debug_println!("thread3 update done");
            Some(shift3)
        },
    )
}

#[test]
#[cfg(not(feature = "disable_slow_tests"))]
fn generic_3threading2a() {
    generic_3thread_ops_b(
        |owner1, shift1, thread| {
            let shift = shift1.upgrade();
            if let Some(mut shift) = shift {
                shift.update(owner1.create(thread));
            }
            Some(shift1)
        },
        |owner2, mut shift2, thread| {
            shift2.update(owner2.create(thread));
            Some(shift2)
        },
        |owner3, mut shift3, thread| {
            shift3.update(owner3.create(thread));
            None
        },
        None,
    );
}
#[test]
fn generic_3threading2b() {
    generic_3thread_ops_b(
        |owner1, shift1, thread| {
            let shift = shift1.upgrade();
            if let Some(mut shift) = shift {
                shift.update(owner1.create(thread));
            }
            None
        },
        |owner2, mut shift2, thread| {
            shift2.update(owner2.create(thread));
            None
        },
        |owner3, mut shift3, thread| {
            shift3.update(owner3.create(thread));
            None
        },
        None,
    );
}
#[test]
fn generic_3threading2c() {
    generic_3thread_ops_c(
        |_owner1, _shift1, _thread| None,
        |_owner2, _shift2, _thread| None,
        |_owner3, _shift3, _thread| None,
        None,
    );
}
#[test]
fn generic_3threading2e() {
    generic_3thread_ops_c(
        |_owner1, shift1, _thread| {
            let _ = shift1.upgrade();
            Some(shift1)
        },
        |_owner2, shift2, _thread| Some(shift2),
        |_owner3, _shift3, _thread| None,
        None,
    );
}

#[test]
fn generic_2threading2b() {
    generic_2thread_ops_b(
        |owner1, shift1, thread| {
            let shift = shift1.upgrade();
            if let Some(mut shift) = shift {
                shift.update(owner1.create(thread));
            }
            Some(shift1)
        },
        |owner2, mut shift2, thread| {
            shift2.update(owner2.create(thread));
            Some(shift2)
        },
        None,
    );
}
#[test]
fn generic_2threading2c() {
    generic_2thread_ops_b(
        |_owner1, shift1, _thread| {
            let _ = shift1.upgrade();
            Some(shift1)
        },
        |owner2, mut shift2, thread| {
            shift2.update(owner2.create(thread));
            None
        },
        None,
    );
}

/// Simple race between non-trivial janitor and Drop
#[test]
fn generic_2threading2d() {
    generic_2thread_ops_d(
        |_owner1, _shift1, _thread| None,
        |owner2, mut shift2, thread| {
            shift2.update(owner2.create(thread));
            None
        },
        None,
    );
}

/// Simple race of drops, where the chain is 3 items long, and one instance points at start,
/// and the other at end.
#[test]
fn generic_2threading2e() {
    generic_2thread_ops_d(
        |_owner1, _shift1, _thread| None,
        |_owner2, mut _shift2, _thread| None,
        None,
    );
}

/// Simple race of drops, where the chain is 3 items long, and one instance points at start,
/// and the other at end.
#[test]
fn generic_2threading2f() {
    generic_2thread_ops_e(
        |_owner1, _shift1, _thread| None,
        |_owner2, mut _shift2, _thread| None,
        None,
    );
}

/// Simple race of drops, where the chain is 3 items long, and one instance points at start,
/// and the other at end.
#[test]
fn generic_2threading2g() {
    generic_2thread_ops_e(
        |owner1, mut shift1, _thread| {
            shift1.update(owner1.create("X"));
            None
        },
        |owner2, mut shift2, _thread| {
            shift2.update(owner2.create("Y"));
            None
        },
        None,
    );
}
