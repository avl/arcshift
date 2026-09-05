use crate::ArcShift;
use core::cell::{Cell, UnsafeCell};
use core::fmt::{Debug, Display, Formatter};
use core::marker::PhantomData;
use core::ops::Deref;

/// ArcShiftCell is like an ArcShift, except that it can be reloaded
/// without requiring 'mut'-access.
/// However, it is not 'Sync'.
///
/// It does not implement 'Deref', but does implement a [`ArcShiftCell::borrow`]-method which returns,
/// a non-threadsafe `ArcShiftCellHandle<T>`. This handle implements Deref, giving access
/// to the pointed to &T. This handle should not be leaked,
/// but if it is leaked, the effect is that whatever value the ArcShiftCell-instance
/// pointed to at that time, will forever leak also.
pub struct ArcShiftCell<T: 'static + ?Sized> {
    // UnsafeCell makes ArcShiftCell invariant, which it needs to be
    inner: UnsafeCell<ArcShift<T>>,
    // Re-entry bookkeeping for `inner`. See `Reentry`.
    reentry: Reentry,
}

/// Tracks re-entrant access to an [`ArcShiftCell`]'s `inner` field.
///
/// Two pieces of state, with distinct meanings:
///
/// * `depth` counts the live `get` frames and outstanding [`ArcShiftCellHandle`]s.
///   While it is non-zero there is (or may be) a `&T` borrowed out of `inner`, so
///   `inner` must not be *reloaded* (that would free the pointed-to node and dangle
///   the reference) - but reading `inner` is fine.
///
/// * `reloading` is raised for the narrower window during which `inner` is borrowed
///   by an in-progress reload, replace or clone. Any of those can free superseded
///   nodes (directly, or via the janitor run during an `ArcShift` advance), running
///   arbitrary user `Drop` code; if that code re-enters this same cell, `inner` is
///   aliased and possibly in a transient state, so *no* access to `inner` is
///   allowed. Re-entering `assign`/`reload` then bail out (error/no-op) and
///   re-entering `borrow`/`get`/`clone`/`make_arcshift` panic.
///
/// Both are restored through RAII guards ([`FrameGuard`], [`ReloadGuard`]) so they
/// are left consistent even if a user `Drop` implementation panics and unwinds
/// through the operation.
struct Reentry {
    depth: Cell<usize>,
    reloading: Cell<bool>,
}

impl Reentry {
    const fn new() -> Reentry {
        Reentry {
            depth: Cell::new(0),
            reloading: Cell::new(false),
        }
    }

    /// True iff no `get` frame or handle is active and no reload is in progress,
    /// i.e. `inner` is fully idle and may be exclusively borrowed.
    fn is_idle(&self) -> bool {
        self.depth.get() == 0 && !self.reloading.get()
    }

    /// True iff exactly one `get` frame or handle is active - used to detect the
    /// outermost frame, the only one allowed to reload.
    fn is_outermost_frame(&self) -> bool {
        self.depth.get() == 1
    }

    /// True while `inner` is `&mut`-borrowed by an in-progress reload/replace.
    fn reload_in_progress(&self) -> bool {
        self.reloading.get()
    }

    /// Enter a `get`/`borrow` frame, bumping `depth`. The returned guard lowers it
    /// again on drop (including on unwind).
    ///
    /// # Panics
    /// Panics if a reload is in progress - the only way to reach this is a user
    /// `Drop` implementation, run while freeing a superseded node, re-entering the
    /// cell. That is unsupported misuse.
    #[inline]
    fn enter_frame(&self) -> FrameGuard<'_> {
        if self.reloading.get() {
            reentrancy_during_reload();
        }
        self.depth.set(self.depth.get() + 1);
        FrameGuard(self)
    }

    /// Mark `inner` as borrowed by a reload/replace/clone for as long as the
    /// returned guard lives. The guard lowers the flag again on drop, including on
    /// unwind (e.g. because a superseded value's `Drop` implementation panicked).
    fn enter_reload(&self) -> ReloadGuard<'_> {
        debug_assert!(
            !self.reloading.get(),
            "reload flag raised recursively - re-entrancy should have been caught earlier"
        );
        self.reloading.set(true);
        ReloadGuard(self)
    }
}

/// Lowers [`Reentry::depth`] by one when dropped. Held for the duration of a `get`
/// call, and embedded in [`ArcShiftCellHandle`] for the handle's whole lifetime.
struct FrameGuard<'a>(&'a Reentry);

impl Drop for FrameGuard<'_> {
    fn drop(&mut self) {
        self.0.depth.set(self.0.depth.get() - 1);
    }
}

/// Lowers [`Reentry::reloading`] when dropped. Using a guard (rather than a plain
/// `set(false)`) ensures the flag is lowered even if the reload unwinds, e.g.
/// because a superseded value's `Drop` implementation panicked.
struct ReloadGuard<'a>(&'a Reentry);

impl Drop for ReloadGuard<'_> {
    fn drop(&mut self) {
        self.0.reloading.set(false);
    }
}

#[cold]
#[inline(never)]
fn reentrancy_during_reload() -> ! {
    panic!(
        "ArcShiftCell was accessed re-entrantly from a Drop implementation that ran \
         while the cell was reloading. This is not supported."
    );
}

/// A handle to the pointed-to value of a ArcShiftCell. This handle should not be leaked,
/// but if it is leaked, the effect is that whatever value the ArcShiftCell-instance
/// pointed to at that time, will forever leak also. All the linked-list nodes from
/// that entry and onward will also leak. So make sure to not leak the handle!
///
/// Leaking the handle does not cause unsoundness and is not UB.
pub struct ArcShiftCellHandle<'a, T: 'static + ?Sized> {
    // Decrements `reentry.depth` when the handle is dropped. Declared first, but
    // because this struct has its own `Drop` impl it still drops *after*
    // `<ArcShiftCellHandle as Drop>::drop` has run - so the last-handle reload there
    // still observes `depth == 1`.
    _frame: FrameGuard<'a>,
    cell: &'a ArcShiftCell<T>,
    // Make sure ArcShiftCellHandle is neither Sync nor Send, and also invariant
    _marker: PhantomData<*mut T>,
}

impl<T: 'static + ?Sized> Drop for ArcShiftCellHandle<'_, T> {
    fn drop(&mut self) {
        // `self._frame` has not dropped yet (it drops after this method returns), so
        // `depth == 1` here means this is the last outstanding handle.
        if self.cell.reentry.is_outermost_frame() && !self.cell.reentry.reload_in_progress() {
            // Last handle going away: reload so superseded nodes can be freed.
            // Freeing nodes runs user `Drop` code, which must not re-enter the cell
            // while `inner` is mutably borrowed - so raise the `reloading` flag for
            // the duration (`_reload` lowers it again, even on unwind). See
            // `ArcShiftCell::borrow`. (A handle cannot actually be created *during* a
            // reload, since `borrow` panics then, so the `reload_in_progress` check
            // above is only defence in depth.)
            let _reload = self.cell.reentry.enter_reload();
            // SAFETY:
            // depth == 1 and reloading was false => no other handle, no outstanding
            // reference => it is safe to obtain a mutable reference to `inner`.
            unsafe { (*self.cell.inner.get()).reload() };
        }
        // `self._frame` now drops: `depth` 1 -> 0, even if the reload above unwound.
    }
}

impl<T: 'static + ?Sized> Deref for ArcShiftCellHandle<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // This must never reload. The returned reference is bound only to `&self`, but
        // `deref` takes `&self` and can be called any number of times, so each returned
        // reference can be held for the whole life of the handle. A reload here would
        // advance `inner` and drop the node that a reference returned by an earlier
        // `deref` call still points into - a use-after-free reachable from safe code.
        //
        // Reloading happens only in `ArcShiftCell::borrow` (for the outermost borrow,
        // i.e. when no other handle - and hence no outstanding reference - exists) and
        // in `ArcShiftCellHandle::drop` when the last handle goes away.
        //
        // SAFETY:
        // Shared access to this UnsafeCell is always allowed. While any handle exists,
        // nothing reloads `inner`, so the pointed-to value stays alive for at least as
        // long as this handle, and therefore as long as the returned reference.
        let inner: &ArcShift<T> = unsafe { &*self.cell.inner.get() };
        inner.shared_non_reloading_get()
    }
}

/// ArcShiftCell cannot be Sync, but there's nothing stopping it from being Send.
///
/// Note that ArcShiftCell *cannot* be Sync, because then multiple threads
/// could call 'get' simultaneously, corrupting the (non-atomic) refcount.
// SAFETY:
// As long as the contents of the cell are not !Send, it is safe to
// send the cell. The object must be uniquely owned to be sent, and
// this is only possible if we're not in a recursive call to
// 'get'. And in this case, the properties of ArcShiftCell are the same
// as ArcShift, and ArcShift is Send (if T is Send + Sync).
//
unsafe impl<T: 'static> Send for ArcShiftCell<T> where T: Send + Sync {}

impl<T: 'static + ?Sized> Clone for ArcShiftCell<T> {
    fn clone(&self) -> Self {
        // A re-entrant clone from a `Drop` running during a reload would alias the
        // borrow of `inner` held by that reload (and might observe a transient state).
        if self.reentry.reload_in_progress() {
            reentrancy_during_reload();
        }
        // `ArcShift::clone` advances an internal copy to the newest node; that
        // advance runs the janitor, which *can* end up running user `Drop` code.
        // Hold the `reloading` flag across it so any such re-entrant access is
        // rejected rather than aliasing `inner`.
        let _reload = self.reentry.enter_reload();
        // SAFETY:
        // Shared access to `inner` is always sound (no other thread can interact
        // with it, and we only read it), and the `reloading` flag rejects the one
        // re-entrant path (user `Drop`) that could alias it.
        let clone = unsafe { &*self.inner.get() }.clone();
        ArcShiftCell {
            inner: UnsafeCell::new(clone),
            reentry: Reentry::new(),
        }
    }
}

/// Error type representing the case that an operation was attempted from within
/// a 'get'-method closure.
pub struct RecursionDetected;

impl Debug for RecursionDetected {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "RecursionDetected")
    }
}

impl Display for RecursionDetected {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "RecursionDetected")
    }
}

impl<T: 'static> ArcShiftCell<T> {
    /// Create a new ArcShiftCell with the given value.
    pub fn new(value: T) -> ArcShiftCell<T> {
        ArcShiftCell::from_arcshift(ArcShift::new(value))
    }
}
impl<T: 'static + ?Sized> ArcShiftCell<T> {
    /// Creates an ArcShiftCell from an ArcShift-instance.
    /// The payload is not cloned, the two pointers keep pointing to the same object.
    pub fn from_arcshift(input: ArcShift<T>) -> ArcShiftCell<T> {
        ArcShiftCell {
            inner: UnsafeCell::new(input),
            reentry: Reentry::new(),
        }
    }

    /// Get a handle to the pointed-to T.
    ///
    /// The handle reloads the cell to the most recent value when it is created
    /// (provided no other handle is currently outstanding). It then acts as a stable
    /// snapshot: dereferencing the handle always yields that same value for the
    /// handle's entire lifetime, even if the value is updated meanwhile. To observe a
    /// later update, drop the handle and call [`ArcShiftCell::borrow`] again.
    ///
    /// Make sure not to leak this handle: See further documentation on
    /// [`ArcShiftCellHandle`]. Leaking the handle will leak resources, but
    /// not cause undefined behaviour.
    ///
    /// # Panics
    /// Panics if called re-entrantly from a `Drop` implementation that runs while
    /// this cell is reloading (i.e. the `Drop` of a value being freed by a reload
    /// tries to `borrow` the same cell). This misuse is not supported.
    #[inline]
    pub fn borrow(&self) -> ArcShiftCellHandle<'_, T> {
        let frame = self.reentry.enter_frame();
        if self.reentry.is_outermost_frame() {
            // Outermost borrow: no other handle exists, so no reference handed out by
            // `<ArcShiftCellHandle as Deref>::deref` is alive. This is the only point
            // (besides dropping the last handle) at which it is sound to reload -
            // `deref` itself must not (see the comment there).
            //
            // The reload can free superseded nodes, running arbitrary user `Drop`
            // code. The `reloading` flag (lowered by `_reload`, even on unwind) makes
            // any re-entrant cell access from that code bail out or panic rather than
            // alias/observe the transiently-borrowed `inner`.
            let _reload = self.reentry.enter_reload();
            // SAFETY:
            // No outstanding handle => no outstanding reference => unique access to `inner`.
            unsafe { &mut *self.inner.get() }.reload();
        }
        ArcShiftCellHandle {
            _frame: frame,
            cell: self,
            _marker: PhantomData,
        }
    }

    /// Get the value pointed to.
    ///
    /// This method is very fast, basically the speed of a regular reference, unless
    /// the value has been modified by calling one of the update-methods.
    ///
    /// This method will do a reload (drop older values which are no longer needed).
    ///
    /// This method is reentrant - you are allowed to call it from within the closure 'f'.
    /// However, only the outermost invocation will cause a reload.
    ///
    /// # Panics
    /// Panics if called re-entrantly from a `Drop` implementation that runs while
    /// this cell is reloading (i.e. the `Drop` of a value being freed by the
    /// outermost invocation's reload tries to access the same cell). This differs
    /// from the supported re-entrancy from within `f`, which runs after the reload
    /// has completed.
    #[inline]
    pub fn get<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let _frame = self.reentry.enter_frame();
        let val = if self.reentry.is_outermost_frame() {
            // Outermost invocation: reload. The reload can free superseded nodes,
            // running user `Drop` code; `_reload` raises the `reloading` flag for the
            // duration (and lowers it even on unwind) so that re-entrant access is
            // rejected rather than aliasing the transiently-borrowed `inner`.
            let _reload = self.reentry.enter_reload();
            // SAFETY:
            // Getting the inner value is safe, no other thread can be accessing it now
            unsafe { &mut *self.inner.get() }.get()
        } else {
            // SAFETY:
            // Getting the inner value is safe, no other thread can be accessing it now
            unsafe { &*self.inner.get() }.shared_non_reloading_get()
        };
        f(val)
        // `_frame` drops here, lowering `depth` - even if `f` panicked.
    }

    /// Assign the given ArcShift to this instance.
    /// This does not copy the value T, it replaces the ArcShift instance of Self
    /// with a clone of 'other'. It does not clone T, only the ArcShift holding it.
    ///
    /// This returns Err if recursion is detected, and has no effect in this case.
    /// Recursion occurs if 'assign' is called from within the closure supplied to
    /// the 'ArcShiftCell::get'-method, or from a `Drop` implementation running while
    /// this cell is reloading.
    pub fn assign(&self, other: &ArcShift<T>) -> Result<(), RecursionDetected> {
        if !self.reentry.is_idle() {
            return Err(RecursionDetected);
        }
        // Both cloning `other` (its advance runs the janitor) and overwriting
        // `inner` (drops the previous `ArcShift`) can free nodes and run user `Drop`
        // code. Raise the `reloading` flag around the whole operation (lowered by
        // `_reload`, even on unwind) so any re-entrant access is rejected.
        let _reload = self.reentry.enter_reload();
        let new_value = other.clone();
        // SAFETY:
        // `reentry` was idle => no outstanding handle/reference, so we have unique
        // access to `inner`.
        *unsafe { &mut *self.inner.get() } = new_value;
        Ok(())
    }
    /// Reload this ArcShiftCell-instance.
    /// This allows heap blocks kept alive by this instance of
    /// ArcShiftCell to be dropped.
    /// Note, this method only works when not called from within a closure
    /// supplied to the 'get' method, or from a `Drop` implementation running while
    /// this cell is already reloading. If such recursion occurs, this method
    /// does nothing.
    pub fn reload(&self) {
        if self.reentry.is_idle() {
            // The reload can free superseded nodes, running user `Drop` code; raise
            // the `reloading` flag around it (lowered by `_reload`, even on unwind)
            // so re-entrant access is rejected.
            let _reload = self.reentry.enter_reload();
            // SAFETY:
            // For 'reload' to be safe, we must be sure that no other execution has a reference
            // to 'self.item', since the value it points to might be dropped.
            // This guarantee is fulfilled, because there is no way to get such a reference
            // other than receiving it in the callback given to 'get'. And for as long as the
            // callback is executing, 'depth' will be != 0.
            unsafe { &mut *self.inner.get() }.reload()
        }
    }
    /// Create an ArcShift-instance pointing to the same value as this cell.
    ///
    /// # Panics
    /// Panics if called re-entrantly from a `Drop` implementation that runs while
    /// this cell is reloading.
    pub fn make_arcshift(&self) -> ArcShift<T> {
        // A re-entrant call from a `Drop` running during a reload would alias the
        // borrow of `inner` held by that reload (and might observe a transient state).
        if self.reentry.reload_in_progress() {
            reentrancy_during_reload();
        }
        // `ArcShift::clone` advances an internal copy to the newest node; that
        // advance runs the janitor, which *can* end up running user `Drop` code.
        // Hold the `reloading` flag across it so any such re-entrant access is
        // rejected rather than aliasing `inner`.
        let _reload = self.reentry.enter_reload();
        // SAFETY:
        // ArcShiftCell is not Sync, so no other thread can interact with `inner`;
        // we only read it, and the `reloading` flag rejects the one re-entrant path
        // (user `Drop`) that could alias it.
        unsafe { &*self.inner.get() }.clone()
    }
}
impl<T: ?Sized + 'static> Debug for ArcShiftCell<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "ArcShiftCell({:?})", &*self.borrow())
    }
}
