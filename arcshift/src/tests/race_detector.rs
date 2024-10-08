//! Fuzzing-test cases which explicit focus on simultaneous operations in different threads

use crate::tests::leak_detection::SpyOwner2;
use crate::tests::{model, model2, InstanceSpy2};
use crate::{atomic, ArcShift, ArcShiftLight};

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
                    f1(&*owner_ref1, shift1, "t1")
                })
                .unwrap();

            let t2 = atomic::thread::Builder::new()
                .name("t2".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    debug_println!(" = On thread t2 =");
                    f2(&*owner_ref2, shift2, "t2")
                })
                .unwrap();

            let t3 = atomic::thread::Builder::new()
                .name("t3".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    debug_println!(" = On thread t3 =");
                    f3(&*owner_ref3, shift3, "t3")
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
            ArcShiftLight<InstanceSpy2>,
            &'static str,
        ) -> Option<ArcShiftLight<InstanceSpy2>>
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
                let shift1 = ArcShiftLight::new(owner.create("orig"));
                let shift2 = shift1.upgrade();
                let shift3 = shift1.upgrade();
                let owner_ref1 = owner.clone();
                let owner_ref2 = owner.clone();
                let owner_ref3 = owner.clone();

                let t1 = atomic::thread::Builder::new()
                    .name("t1".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t1 =");
                        f1(&*owner_ref1, shift1, "t1")
                    })
                    .unwrap();

                let t2 = atomic::thread::Builder::new()
                    .name("t2".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t2 =");
                        f2(&*owner_ref2, shift2, "t2")
                    })
                    .unwrap();

                let t3 = atomic::thread::Builder::new()
                    .name("t3".to_string())
                    .stack_size(1_000_000)
                    .spawn(move || {
                        debug_println!(" = On thread t3 =");
                        f3(&*owner_ref3, shift3, "t3")
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
#[cfg(not(feature = "disable_slow_tests"))]
#[test]
fn generic_3threading_b_all() {
    generic_3threading_b_all_impl(0, 0, 0, None);
}

fn generic_3threading_b_all_impl(skip1: usize, skip2: usize, skip3: usize, repro: Option<&str>) {
    let ops1: Vec<
        fn(
            &SpyOwner2,
            ArcShiftLight<InstanceSpy2>,
            &'static str,
        ) -> Option<ArcShiftLight<InstanceSpy2>>,
    > = vec![
        |_, shift, _| {
            _ = shift.upgrade();
            Some(shift)
        },
        |_, shift, _| {
            _ = shift.clone();
            Some(shift)
        },
        |_, shift, _| {
            _ = shift.upgrade();
            None
        },
        |_, shift, _| Some(shift),
        |owner, shift, thread| {
            shift.update_shared(owner.create(thread));
            Some(shift)
        },
        |_, mut shift, _| {
            shift.reload();
            Some(shift)
        },
        |_, _, _| None,
    ];
    let ops23: Vec<
        fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>,
    > = vec![
        |owner, shift, thread| {
            shift.update_shared(owner.create(thread));
            Some(shift)
        },
        |owner, mut shift, thread| {
            shift.update(owner.create(thread));
            Some(shift)
        },
        |_owner, mut shift, _thread| {
            std::hint::black_box(shift.get());
            Some(shift)
        },
        |_owner, shift, _thread| {
            std::hint::black_box(shift.shared_get());
            Some(shift)
        },
        |_owner, mut shift, _thread| {
            shift.reload();
            Some(shift)
        },
        |_owner, shift, _thread| {
            std::hint::black_box(shift.try_into_inner());
            None
        },
        |_owner, _shift, _thread| None,
    ];
    for (_n1, op1) in ops1.iter().enumerate().skip(skip1) {
        for (_n2, op2) in ops23.iter().enumerate().skip(skip2) {
            for (_n3, op3) in ops23.iter().enumerate().skip(skip3) {
                {
                    println!("\n");
                    println!(
                        " ===================== {} {} {} ======================",
                        _n1, _n2, _n3
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
    let ops: Vec<
        fn(&SpyOwner2, ArcShift<InstanceSpy2>, &'static str) -> Option<ArcShift<InstanceSpy2>>,
    > = vec![
        |owner, shift, thread| {
            shift.update_shared(owner.create(thread));
            Some(shift)
        },
        |owner, mut shift, thread| {
            shift.update(owner.create(thread));
            Some(shift)
        },
        |_owner, mut shift, _thread| {
            std::hint::black_box(shift.get());
            Some(shift)
        },
        |_owner, shift, _thread| {
            std::hint::black_box(shift.shared_get());
            Some(shift)
        },
        |_owner, mut shift, _thread| {
            shift.reload();
            Some(shift)
        },
        |_owner, shift, _thread| {
            std::hint::black_box(shift.try_into_inner());
            None
        },
        |_owner, _shift, _thread| None,
    ];
    for (n1, op1) in ops.iter().enumerate() {
        for (n2, op2) in ops.iter().enumerate() {
            for (n3, op3) in ops.iter().enumerate() {
                {
                    println!("========= {} {} {} ==========", n1, n2, n3);
                }
                generic_3thread_ops_a(*op1, *op2, *op3)
            }
        }
    }
}

#[test]
fn generic_3threading1() {
    generic_3thread_ops_a(
        |owner1, shift1, thread| {
            shift1.update_shared(owner1.create(thread));
            Some(shift1)
        },
        |owner2, mut shift2, thread| {
            shift2.update(owner2.create(thread));
            Some(shift2)
        },
        |owner3, shift3, thread| {
            shift3.update_shared(owner3.create(thread));
            Some(shift3)
        },
    )
}

#[test]
fn generic_3threading2() {
    generic_3thread_ops_b(
        |owner1, shift1, thread| {
            shift1.update_shared(owner1.create(thread));
            Some(shift1)
        },
        |owner2, mut shift2, thread| {
            shift2.update(owner2.create(thread));
            Some(shift2)
        },
        |owner3, mut shift3, thread| {
            shift3.update(owner3.create(thread));
            Some(shift3)
        },
        None,
    );
}
