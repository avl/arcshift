//! Regression test: a reference obtained from an `ArcShiftCellHandle` (via `Deref`)
//! must stay valid for the whole lifetime of the handle, even across further
//! `deref` calls and concurrent-looking updates from other handles to the chain.
//!
//! Before the fix, `<ArcShiftCellHandle as Deref>::deref` reloaded the inner
//! `ArcShift` in the `recursion == 1` case. Because `deref` takes `&self` it can be
//! called repeatedly, and every returned reference is bound only to the handle
//! borrow, so a later `deref` could drop the node an earlier reference still
//! pointed into -> use-after-free in safe code (Miri: "dangling reference
//! (use-after-free)").
//!
//! The fix moves reloading into `ArcShiftCell::borrow` (only when no handle is
//! outstanding) and keeps `deref` non-reloading.

use arcshift::cell::ArcShiftCell;
use arcshift::ArcShift;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

static LIVE: AtomicUsize = AtomicUsize::new(0);

struct Spy {
    tag: u64,
    heap: Arc<u64>,
}
impl Spy {
    fn new(tag: u64) -> Spy {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Spy {
            tag,
            heap: Arc::new(tag),
        }
    }
    fn tag(&self) -> u64 {
        // Touch the heap allocation so any use-after-free is unambiguous under Miri/ASAN.
        assert_eq!(*self.heap, self.tag);
        self.tag
    }
}
impl Drop for Spy {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

#[test]
fn cell_handle_reference_survives_later_deref_and_update() {
    let mut root = ArcShift::new(Spy::new(1));
    let cell = ArcShiftCell::from_arcshift(root.clone());

    let h = cell.borrow();

    // Deref #1: reference into node A (tag 1), bound to the borrow of `h`.
    let a: &Spy = &*h;
    assert_eq!(a.tag(), 1);
    assert_eq!(LIVE.load(Ordering::SeqCst), 1);

    // Make node A stale: `root` advances to node B; node A is now kept alive only by
    // the cell/handle.
    root.update(Spy::new(2));
    assert_eq!(LIVE.load(Ordering::SeqCst), 2);

    // Deref #2 on the same handle. This must NOT reload (doing so would free node A).
    let b: &Spy = &*h;

    // The handle is a stable snapshot: it still sees node A.
    assert_eq!(b.tag(), 1);
    // And the reference from deref #1 is still valid.
    assert_eq!(a.tag(), 1);
    assert_eq!(
        LIVE.load(Ordering::SeqCst),
        2,
        "node A must remain alive while a handle references it"
    );

    // Dropping the last handle reloads: node A's payload is freed now.
    drop(h);
    assert_eq!(LIVE.load(Ordering::SeqCst), 1);

    // A fresh borrow (no handle outstanding) reloads and observes the update.
    assert_eq!(cell.borrow().tag(), 2);
    assert_eq!(LIVE.load(Ordering::SeqCst), 1); // only node B (tag 2), still held by `cell`

    drop(root);
    drop(cell);
    assert_eq!(LIVE.load(Ordering::SeqCst), 0);
}
