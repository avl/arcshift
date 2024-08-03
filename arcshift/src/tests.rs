#![allow(dead_code)]
#![allow(unused_imports)]
use super::*;
use crossbeam_channel::bounded;
use std::alloc::Layout;
use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::hint::black_box;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use leak_detection::{InstanceSpy2, SpyOwner2, InstanceSpy};
use rand::prelude::StdRng;
use rand::{Rng, SeedableRng};


mod leak_detection;
mod race_detector;
mod custom_fuzz;


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

#[cfg(feature = "shuttle")]
fn model(x: impl Fn() + 'static + Send + Sync) {
    shuttle::check_random(x, 5000);
}
#[cfg(feature = "shuttle")]
fn model2(x: impl Fn() + 'static + Send + Sync, repro: Option<&str>) {
    if let Some(repro) = repro {
        shuttle::replay(x, repro);
    } else {
        shuttle::check_random(x, 5000);
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
fn simple_get_mut() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        // Uniquely owned values can be modified using 'try_get_mut'.
        assert_eq!(*shift.try_get_mut().unwrap(), 42);
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


#[test]
fn simple_clone_light() {
    model(|| {
        let shift = ArcShiftLight::new(42u32);
        _ = shift.clone();
    });
}
#[test]
fn simple_update_light() {
    model(|| {
        let mut shift = ArcShiftLight::new(42u32);
        shift.update(43);
        assert_eq!(*shift.upgrade().get(), 43);
    });
}
#[test]
fn simple_update_box_light() {
    model(|| {
        let mut shift = ArcShiftLight::new(42u32);
        shift.update_box(Box::new(43));
        assert_eq!(*shift.upgrade().get(), 43);
    });
}

#[test]
// There's no point in running this test under shuttle/loom,
// and since it can take some time, let's just disable it.
#[cfg(not(any(loom, feature="shuttle")))]
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
        shift.update_shared(43);
        assert_eq!(*shift.get(), 43u32);
    });
}
#[test]
fn simple_update_boxed() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
        shift.update_shared_box(Box::new(43));
        assert_eq!(*shift.get(), 43u32);
    });
}

#[test]
fn simple_update2() {
    model(|| {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
        shift.update_shared(43);
        shift.update_shared(44);
        shift.update_shared(45);
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
fn simple_update4() {
    model(|| {
        let mut shiftlight = ArcShiftLight::new(1);
        shiftlight.update(2);
        assert_eq!(*shiftlight.upgrade().get(), 2);

        shiftlight.update_shared(3);
        assert_eq!(*shiftlight.upgrade().get(), 3);

        shiftlight.update_box(Box::new(4));
        assert_eq!(*shiftlight.upgrade().get(), 4);

        shiftlight.update_shared_box(Box::new(5));
        assert_eq!(*shiftlight.upgrade().get(), 5);
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
        let shiftlight = ArcShiftLight::new(InstanceSpy::new(count.clone()));

        debug_println!("==== running shift.get() = ");
        let mut shift = shiftlight.upgrade();
        debug_println!("==== running arc.update() = ");
        shift.update(InstanceSpy::new(count.clone()));

        debug_println!("==== Instance count: {}", count.load(Ordering::SeqCst));
        assert_eq!(count.load(Ordering::SeqCst), 1); // The 'ArcShiftLight' should *not* keep any version alive
        debug_println!("==== drop arc =");
        drop(shift);
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
        let shift = ArcShiftLight::new(InstanceSpy::new(count.clone()));
        let mut arc = shift.upgrade();
        for _ in 0..10 {
            arc.update(InstanceSpy::new(count.clone()));
            debug_println!("Instance count: {}", count.load(Ordering::SeqCst));
            assert_eq!(count.load(Ordering::Relaxed), 1); // The 'ArcShiftLight' should *not* keep any version alive
        }
        drop(arc);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        drop(shift);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    });
}
#[test]
fn simple_upgrade3b() {
    model(|| {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let shift = ArcShiftLight::new(InstanceSpy::new(count.clone()));
        let mut arc = shift.upgrade();
        for _ in 0..10 {
            arc.update(InstanceSpy::new(count.clone()));
            black_box(arc.clone());
            debug_println!("Instance count: {}", count.load(Ordering::SeqCst));
            assert_eq!(count.load(Ordering::Relaxed), 1); // The 'ArcShiftLight' should *not* keep any version alive
        }
        drop(arc);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        drop(shift);
        assert_eq!(count.load(Ordering::Relaxed), 0);
    });
}

#[test]
fn simple_upgrade4() {
    model(|| {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let shiftlight = ArcShiftLight::new(InstanceSpy::new(count.clone()));
        let shift = shiftlight.upgrade();
        debug_println!("== Calling update_shared ==");
        shift.update_shared(InstanceSpy::new(count.clone()));
        debug_println!("== Calling shared_get ==");
        _ = shift.shared_get();
        debug_println!("== Calling update_shared ==");
        shift.update_shared(InstanceSpy::new(count.clone()));
        debug_println!("== Calling drop(shift) ==");
        drop(shift);

        assert_eq!(count.load(Ordering::Relaxed), 1);
    });
}
#[test]
fn simple_upgrade5() {
    model(|| {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut shiftlight = ArcShiftLight::new(InstanceSpy::new(count.clone()));
        assert_eq!(count.load(Ordering::Relaxed), 1);
        let shift = shiftlight.upgrade();
        debug_println!("== Calling update_shared ==");
        shift.update_shared(InstanceSpy::new(count.clone()));
        drop(shift);
        debug_println!("== Calling shared_get ==");
        shiftlight.reload();
        assert_eq!(count.load(Ordering::Relaxed), 1);
    });
}
#[test]
fn simple_upgrade6() {
    model(|| {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let mut shiftlight = ArcShiftLight::new(InstanceSpy::new(count.clone()));
            assert_eq!(shiftlight.get_internal_node_count(), 1);
            debug_println!("== Calling update_shared ==");
            shiftlight.update_shared(InstanceSpy::new(count.clone()));
            assert_eq!(shiftlight.get_internal_node_count(), 2);
            debug_println!("== Calling shared_get ==");
            shiftlight.reload();
            assert_eq!(shiftlight.get_internal_node_count(), 1);

            assert_eq!(count.load(Ordering::Relaxed), 1);
        }
        assert_eq!(count.load(Ordering::Relaxed), 0);
    });
}

#[test]
fn simple_upgrade4b() {
    model(|| {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let shiftlight = ArcShiftLight::new(InstanceSpy::new(count.clone()));
            let _shiftlight1 = shiftlight.clone();
            let _shiftlight2 = shiftlight.clone();
            let _shiftlight3 = shiftlight.clone();
            let _shiftlight4 = shiftlight.clone();
            let _shiftlight5 = shiftlight.clone(); //Verify that early drop still happens with several light references (silences a cargo mutants-test :-) )

            let mut shift = shiftlight.upgrade();
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
            let mut shiftlight = ArcShiftLight::new(InstanceSpy::new(count.clone()));
            let _shiftlight1 = shiftlight.clone();
            let _shiftlight2 = shiftlight.clone();
            let _shiftlight3 = shiftlight.clone();
            let _shiftlight4 = shiftlight.clone();
            let _shiftlight5 = shiftlight.clone(); //Verify that early drop still happens with several light references (silences a cargo mutants-test :-) )

            debug_println!("== Calling update_shared ==");
            shiftlight.update(InstanceSpy::new(count.clone()));
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
        let shift1 = shift.clone();
        let mut shift2 = shift1.clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                shift1.update_shared(43);
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
            let shiftlight = ArcShiftLight::new(owner.create("orig"));
            let mut shift = shiftlight.upgrade();
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
fn simple_threading2b() {
    model(|| {
        let shift1 = ArcShiftLight::new(42u32);
        let mut shift2 = shift1.upgrade();
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
        let shift1 = ArcShiftLight::new(42u32);
        let mut shift2 = shift1.upgrade();
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
                shift1.update_shared(43);
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
        let shift1 = std::sync::Arc::new(ArcShift::new(42u32));
        let shift2 = std::sync::Arc::clone(&shift1);
        let shift3 = (*shift1).clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                std::hint::black_box(shift1.update_shared(43));
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                std::hint::black_box(shift2.update_shared(44));
                debug_println!(" = drop t2 =");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t3 =");
                std::hint::black_box(shift3.update_shared(45));
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
        let shift1 = std::sync::Arc::new(ArcShiftLight::new(42u32));
        let shift2 = std::sync::Arc::clone(&shift1);
        let shift3 = (*shift1).upgrade();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                std::hint::black_box(shift1.upgrade());
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                std::hint::black_box(shift2.update_shared(44));
                debug_println!(" = drop t2 =");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t3 =");
                std::hint::black_box(shift3.update_shared(45));
                debug_println!(" = drop t3 =");
            })
            .unwrap();
        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        _ = t3.join().unwrap();
    });
}
#[cfg(not(feature="disable_slow_tests"))]
#[test]
fn simple_threading4a() {
    model(|| {
        let shift1 = std::sync::Arc::new(ArcShift::new(42u32));
        let shift2 = std::sync::Arc::clone(&shift1);
        let shift3 = std::sync::Arc::clone(&shift1);
        let mut shift4 = (*shift1).clone();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                shift1.update_shared(43);
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
                shift3.update_shared(44);
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

#[cfg(not(feature="disable_slow_tests"))]
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
                shift1.update_shared(43);
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
                shift3.update_shared(44);
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

#[cfg(not(feature="disable_slow_tests"))]
#[test]
fn simple_threading4c() {
    model(|| {
        let count = std::sync::Arc::new(SpyOwner2::new());
        {
            let shift1 = std::sync::Arc::new(ArcShift::new(count.create("orig")));
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
                    shift1.update_shared(count1.create("t1val"));
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
                    shift3.update_shared(count2.create("t3val"));

                    let _dbgval = get_next_and_state(shift3.item).load(Ordering::SeqCst);
                    verify_item(shift3.item);
                    debug_println!("Checkt34c: {:?} next: {:?}", shift3.item, _dbgval);
                    debug_println!(" = drop t3 =");
                })
                .unwrap();
            let t4 = atomic::thread::Builder::new()
                .name("t4".to_string())
                .stack_size(1_000_00)
                .spawn(move || {
                    debug_println!(" = On thread t4 = {:?}", std::thread::current().id());
                    let shift4 = &*shift4;
                    verify_item(shift4.item);
                    debug_println!(
                        "Checkt44c: {:?} next: {:?}",
                        shift4.item,
                        get_next_and_state(shift4.item).load(Ordering::SeqCst)
                    );
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
#[cfg(not(feature="disable_slow_tests"))]
#[test]
fn simple_threading4d() {
    model(|| {
        let count = std::sync::Arc::new(SpyOwner2::new());
        {
            let count3 = count.clone();
            let shift1 = std::sync::Arc::new(ArcShiftLight::new(count.create("orig")));

            let shift2 = shift1.upgrade();
            let shift3 = shift1.upgrade();
            let mut shift4 = shift1.upgrade();
            let t1 = atomic::thread::Builder::new()
                .name("t1".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    debug_println!(" = On thread t1 =");
                    black_box(shift1.upgrade());
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
                    shift3.update_shared(count3.create("t3val"));
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
#[cfg(not(feature="disable_slow_tests"))]
#[test]
fn simple_threading4e() {
    model(|| {
        let shift = std::sync::Arc::new(ArcShiftLight::new(42u32));
        let shift1 = shift.upgrade();
        let shift2 = shift.upgrade();
        let shift3 = shift.upgrade();
        let shift4 = shift.upgrade();
        let t1 = atomic::thread::Builder::new()
            .name("t1".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t1 =");
                shift1.update_shared(43);
                debug_println!(" = drop t1 =");
            })
            .unwrap();

        let t2 = atomic::thread::Builder::new()
            .name("t2".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t2 =");
                shift2.update_shared(44);
                debug_println!(" = drop t2 =");
            })
            .unwrap();

        let t3 = atomic::thread::Builder::new()
            .name("t3".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t3 =");
                shift3.update_shared(45);
                debug_println!(" = drop t3 =");
            })
            .unwrap();
        let t4 = atomic::thread::Builder::new()
            .name("t4".to_string())
            .stack_size(1_000_000)
            .spawn(move || {
                debug_println!(" = On thread t4 =");
                shift4.update_shared(46);
                debug_println!(" = drop t4 =");
            })
            .unwrap();

        _ = t1.join().unwrap();
        _ = t2.join().unwrap();
        _ = t3.join().unwrap();
        _ = t4.join().unwrap();
    });
}

