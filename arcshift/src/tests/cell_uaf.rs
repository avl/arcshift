use crate::cell::ArcShiftCell;
use crate::ArcShift;
use std::cell::Cell as StdCell;
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
    let a: &Spy = &h;
    assert_eq!(a.tag(), 1);
    assert_eq!(LIVE.load(Ordering::SeqCst), 1);

    // Make node A stale: `root` advances to node B; node A is now kept alive only by
    // the cell/handle.
    root.update(Spy::new(2));
    assert_eq!(LIVE.load(Ordering::SeqCst), 2);

    // Deref #2 on the same handle. This must NOT reload (doing so would free node A).
    let b: &Spy = &h;

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

// --- Regression: a `Drop` that re-enters the cell while it is reloading ---------
//
// When a reload frees a superseded node, that node's payload `Drop` runs while the
// cell holds `&mut inner`. If that `Drop` re-enters the same cell, it must not be
// allowed to observe/alias `inner`. `borrow`/`get`/`make_arcshift`/`clone` panic;
// `assign`/`reload` become no-ops/errors. These tests use per-scenario `Arc`
// trackers rather than the shared `LIVE` counter so they are safe to run in
// parallel with each other and with the test above.

#[derive(Clone, Copy, PartialEq, Debug)]
enum OnDrop {
    Nothing,
    Borrow,
    Get,
    MakeArcshift,
    CloneCell,
    Reload,
    Assign,
}

std::thread_local! {
    // Raw pointer to the cell currently under test, valid only for the test body.
    static REENTRY_CELL: StdCell<*const ArcShiftCell<Reenter>> =
        const { StdCell::new(std::ptr::null()) };
}

struct CellPtrGuard;
impl Drop for CellPtrGuard {
    fn drop(&mut self) {
        REENTRY_CELL.with(|p| p.set(std::ptr::null()));
    }
}
fn install_cell(cell: &ArcShiftCell<Reenter>) -> CellPtrGuard {
    REENTRY_CELL.with(|p| p.set(cell as *const _));
    CellPtrGuard
}

struct Reenter {
    on_drop: OnDrop,
    // Kept alive exactly as long as this payload; lets a test observe that this
    // specific node was dropped.
    _tracker: Arc<()>,
}
impl Reenter {
    fn new(on_drop: OnDrop, tracker: Arc<()>) -> Reenter {
        Reenter {
            on_drop,
            _tracker: tracker,
        }
    }
}
impl Drop for Reenter {
    fn drop(&mut self) {
        let ptr = REENTRY_CELL.with(|p| p.get());
        if ptr.is_null() || self.on_drop == OnDrop::Nothing {
            return;
        }
        // SAFETY: test-only. The pointer is set to a live cell for the whole test
        // body (and cleared afterwards by `CellPtrGuard`).
        let cell: &ArcShiftCell<Reenter> = unsafe { &*ptr };
        match self.on_drop {
            OnDrop::Nothing => unreachable!(),
            OnDrop::Borrow => {
                let _ = cell.borrow();
            }
            OnDrop::Get => cell.get(|_| {}),
            OnDrop::MakeArcshift => {
                let _ = cell.make_arcshift();
            }
            OnDrop::CloneCell => {
                let _ = cell.clone();
            }
            OnDrop::Reload => cell.reload(),
            OnDrop::Assign => {
                // Must report recursion and have no effect.
                let dummy = ArcShift::new(Reenter::new(OnDrop::Nothing, Arc::new(())));
                assert!(cell.assign(&dummy).is_err());
            }
        }
    }
}

/// Drive a cell so that a reload frees node A, whose payload's `Drop` does `action`.
/// Returns node A's tracker, so the caller can check it was dropped.
fn trigger_reload_freeing_reentrant_node(action: OnDrop) -> Arc<()> {
    let tracker = Arc::new(());
    let mut root = ArcShift::new(Reenter::new(action, tracker.clone()));
    let cell = ArcShiftCell::from_arcshift(root.clone());
    let _guard = install_cell(&cell);

    // Realize the cell at node A.
    cell.get(|_| {});
    // Node B; node A is now stale but still kept alive by `cell`.
    root.update(Reenter::new(OnDrop::Nothing, Arc::new(())));
    // Reload: frees node A, running its payload's `Drop` while `inner` is borrowed.
    let _ = cell.borrow();

    drop(root);
    drop(cell);
    tracker
}

#[test]
#[should_panic(expected = "accessed re-entrantly")]
fn cell_reentrant_borrow_from_drop_during_reload_panics() {
    trigger_reload_freeing_reentrant_node(OnDrop::Borrow);
}

#[test]
#[should_panic(expected = "accessed re-entrantly")]
fn cell_reentrant_get_from_drop_during_reload_panics() {
    trigger_reload_freeing_reentrant_node(OnDrop::Get);
}

#[test]
#[should_panic(expected = "accessed re-entrantly")]
fn cell_reentrant_make_arcshift_from_drop_during_reload_panics() {
    trigger_reload_freeing_reentrant_node(OnDrop::MakeArcshift);
}

#[test]
#[should_panic(expected = "accessed re-entrantly")]
fn cell_reentrant_clone_from_drop_during_reload_panics() {
    trigger_reload_freeing_reentrant_node(OnDrop::CloneCell);
}

#[test]
fn cell_reentrant_reload_from_drop_during_reload_is_noop() {
    let tracker = trigger_reload_freeing_reentrant_node(OnDrop::Reload);
    assert_eq!(
        Arc::strong_count(&tracker),
        1,
        "node A's payload should have been dropped without panicking"
    );
}

#[test]
fn cell_reentrant_assign_from_drop_during_reload_errors() {
    let tracker = trigger_reload_freeing_reentrant_node(OnDrop::Assign);
    assert_eq!(
        Arc::strong_count(&tracker),
        1,
        "node A's payload should have been dropped without panicking"
    );
}

// --- A panic *inside* a `get` closure must not poison the cell -----------------
//
// The frame counter is restored by an RAII guard, so an unwinding closure leaves
// `depth` at 0 and the cell keeps working (reloads still happen, `assign` still
// succeeds). Before the guard, the manual decrement was skipped on unwind and the
// cell was permanently stuck.

#[test]
fn cell_get_closure_panic_does_not_poison() {
    // Node A carries `a_tracker`; the cell realizes at A, then A goes stale.
    let a_tracker = Arc::new(());
    let mut root = ArcShift::new(Reenter::new(OnDrop::Nothing, a_tracker.clone()));
    let cell = ArcShiftCell::from_arcshift(root.clone());
    cell.get(|_| {}); // realize the cell at node A

    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cell.get(|_| panic!("boom"));
    }));
    assert!(panicked.is_err());

    // Node B; node A is now stale but still kept alive by the cell.
    root.update(Reenter::new(OnDrop::Nothing, Arc::new(())));
    assert_eq!(Arc::strong_count(&a_tracker), 2); // test + node A's payload

    // `assign` must not spuriously report recursion (the frame counter was
    // restored by its guard despite the unwind)...
    let newer = ArcShift::new(Reenter::new(OnDrop::Nothing, Arc::new(())));
    cell.assign(&newer)
        .expect("cell must not be poisoned by a panicking `get` closure");
    // ...and replacing the cell's value frees node A.
    assert_eq!(
        Arc::strong_count(&a_tracker),
        1,
        "node A should have been freed - cell still works after a closure panic"
    );
}

// --- `clone` / `make_arcshift` hold the `reloading` flag (defence in depth) ----
//
// `ArcShift::clone` advances an internal copy to the newest node, running the
// janitor - which can, in principle, run user `Drop` code. Both cell methods now
// hold the flag across that call. These tests check the happy path still works
// with superseded nodes present (the flag must not spuriously trip, and the
// produced value must be the newest one).

#[test]
fn cell_clone_with_superseded_nodes() {
    let stale_tracker = Arc::new(());
    let mut root = ArcShift::new(Reenter::new(OnDrop::Nothing, stale_tracker.clone()));
    let cell = ArcShiftCell::from_arcshift(root.clone());
    cell.get(|_| {}); // realize `cell` at the first node
    root.update(Reenter::new(OnDrop::Nothing, Arc::new(()))); // first node now stale

    let cloned = cell.clone();
    // The clone is an independent, working cell pointing at the newest value.
    cloned.reload();
    cloned.get(|_| {});
    cloned.assign(&root).expect("cloned cell must be usable");

    // Original still works too, and reloading it frees the stale node.
    cell.reload();
    drop(root);
    drop(cell);
    drop(cloned);
    assert_eq!(Arc::strong_count(&stale_tracker), 1);
}

#[test]
fn cell_make_arcshift_with_superseded_nodes() {
    let stale_tracker = Arc::new(());
    let mut root = ArcShift::new(Reenter::new(OnDrop::Nothing, stale_tracker.clone()));
    let cell = ArcShiftCell::from_arcshift(root.clone());
    cell.get(|_| {}); // realize `cell` at the first node
    root.update(Reenter::new(OnDrop::Nothing, Arc::new(()))); // first node now stale

    // `make_arcshift` must not panic and yields a working `ArcShift`.
    let mut handed_out = cell.make_arcshift();
    handed_out.reload();

    // Reloading the cell frees the stale node even though `handed_out` is alive.
    cell.reload();
    drop(root);
    assert_eq!(
        Arc::strong_count(&stale_tracker),
        1,
        "stale node should be freed; `make_arcshift` did not pin it"
    );
    drop(cell);
    drop(handed_out);
}
