use crate::ArcShift;
use std::boxed::Box;
use std::sync::Arc;

enum Payload {
    /// Panics on drop; carries an `Arc` probe so a leak is visible via strong_count.
    Boom(#[allow(dead_code)] Arc<()>),
    Quiet(#[allow(dead_code)] Box<u32>),
}
impl Drop for Payload {
    fn drop(&mut self) {
        if let Payload::Boom(_) = self {
            panic!("boom");
        }
    }
}

fn catching(f: impl FnOnce()) {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    assert!(r.is_err(), "expected the destructor panic to propagate");
}

#[test]
fn update_then_panic_then_drop() {
    catching(|| {
        let mut a = ArcShift::new(Payload::Boom(Arc::new(())));
        a.update(Payload::Quiet(Box::new(2)));
        let _ = &a;
    });
}

#[test]
fn get_then_panic_then_drop() {
    catching(|| {
        let mut a = ArcShift::new(Payload::Boom(Arc::new(())));
        let mut b = a.clone();
        b.update(Payload::Quiet(Box::new(2)));
        drop(b);
        let _ = a.get();
        let _ = &a;
    });
}

#[test]
fn reload_then_panic_then_drop() {
    catching(|| {
        let mut a = ArcShift::new(Payload::Boom(Arc::new(())));
        let mut b = a.clone();
        b.update(Payload::Quiet(Box::new(2)));
        drop(b);
        a.reload();
        let _ = &a;
    });
}

#[test]
fn rcu_then_panic_then_drop() {
    catching(|| {
        let mut a = ArcShift::new(Payload::Boom(Arc::new(())));
        a.rcu(|_| Payload::Quiet(Box::new(2)));
        let _ = &a;
    });
}

#[test]
fn update_box_then_panic_then_drop() {
    catching(|| {
        let mut a = ArcShift::new(Payload::Boom(Arc::new(())));
        a.update_box(Box::new(Payload::Quiet(Box::new(2))));
        let _ = &a;
    });
}

/// After the panic is caught, the superseded node (and its payload) must still
/// have been freed -- no leak from the panicking advance. Uses a local `Arc`
/// probe so it is safe to run in parallel with the other tests.
#[test]
fn update_panic_does_not_leak() {
    let probe = Arc::new(());
    let p = probe.clone();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let mut a = ArcShift::new(Payload::Boom(p));
        a.update(Payload::Quiet(Box::new(2)));
        let _ = &a;
    }));
    assert!(r.is_err());
    assert_eq!(
        Arc::strong_count(&probe),
        1,
        "superseded payload leaked across the panic"
    );
}
