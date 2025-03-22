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
/// pointed to at that time, will forever leak also. All the linked-list nodes from
/// that entry and onward will also leak. So make sure to not leak the handle!
pub struct ArcShiftCell<T: 'static + ?Sized> {
    inner: UnsafeCell<ArcShift<T>>,
    recursion: Cell<usize>,
}

/// A handle to the pointed-to value of a ArcShiftCell. This handle should not be leaked,
/// but if it is leaked, the effect is that whatever value the ArcShiftCell-instance
/// pointed to at that time, will forever leak also. All the linked-list nodes from
/// that entry and onward will also leak. So make sure to not leak the handle!
///
/// Leaking the handle does not cause unsoundness and is not UB.
pub struct ArcShiftCellHandle<'a, T: 'static + ?Sized> {
    cell: &'a ArcShiftCell<T>,
    // Make sure ArcShiftCellHandle is neither Sync nor Send
    _marker: PhantomData<*mut T>,
}

impl<T: 'static + ?Sized> Drop for ArcShiftCellHandle<'_, T> {
    fn drop(&mut self) {
        let mut rec = self.cell.recursion.get();
        rec -= 1;
        if rec == 0 {
            // SAFETY:
            // There are no references to 'inner', so it's safe to obtain
            // a mutable reference.
            unsafe { (*self.cell.inner.get()).reload() };
        }
        self.cell.recursion.set(rec);
    }
}

impl<T: 'static + ?Sized> Deref for ArcShiftCellHandle<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        if self.cell.recursion.get() == 1 {
            // SAFETY:
            // We're the only owner of the ArcShiftCell, and can thus get mutable access.
            let inner: &mut ArcShift<T> = unsafe { &mut *self.cell.inner.get() };
            inner.get()
        } else {
            // SAFETY:
            // Shared access to this UnsafeCell is always allowed.
            // Actual mutable references to the 'inner' never live long enough
            // to be visible by the user of this module.
            let inner: &ArcShift<T> = unsafe { &*self.cell.inner.get() };
            inner.shared_get()
        }
    }
}

/// ArcShiftCell cannot be Sync, but there's nothing stopping it from being Send.
/// SAFETY:
/// As long as the contents of the cell are not !Send, it is safe to
/// send the cell. The object must be uniquely owned to be sent, and
/// this is only possible if we're not in a recursive call to
/// 'get'. And in this case, the properties of ArcShiftCell are the same
/// as ArcShift, and ArcShift is Send.
///
/// Note that ArcShiftCell *cannot* be Sync, because then multiple threads
/// could call 'get' simultaneously, corrupting the (non-atomic) refcount.
unsafe impl<T: 'static> Send for ArcShiftCell<T> where T: Send {}

impl<T: 'static + ?Sized> Clone for ArcShiftCell<T> {
    fn clone(&self) -> Self {
        // SAFETY:
        // Accessing the inner value is safe, since we know no other thread
        // can be interacting with it. And even if we're in a recursive call to
        // 'get', we're not mutating the value and cloning it is thus perfectly safe.
        let clone = unsafe { &mut *self.inner.get() }.clone();
        ArcShiftCell {
            inner: UnsafeCell::new(clone),
            recursion: Cell::new(0),
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
            recursion: Cell::new(0),
        }
    }

    /// Get a handle to the pointed to T.
    ///
    /// Make sure not to leak this handle: See further documentation on
    /// [`ArcShiftCellHandle`]. Leaking the handle will leak resources, but
    /// not cause undefined behaviour.
    pub fn borrow(&self) -> ArcShiftCellHandle<T> {
        self.recursion.set(self.recursion.get() + 1);
        ArcShiftCellHandle {
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
    pub fn get(&self, f: impl FnOnce(&T)) {
        self.recursion.set(self.recursion.get() + 1);
        let val = if self.recursion.get() == 1 {
            // SAFETY:
            // Getting the inner value is safe, no other thread can be accessing it now
            unsafe { &mut *self.inner.get() }.get()
        } else {
            // SAFETY:
            // Getting the inner value is safe, no other thread can be accessing it now
            unsafe { &*self.inner.get() }.shared_get()
        };
        f(val);
        self.recursion.set(self.recursion.get() - 1);
    }

    /// Assign the given ArcShift to this instance.
    /// This does not copy the value T, it replaces the ArcShift instance of Self
    /// with a clone of 'other'. It does not clone T, only the ArcShift holding it.
    ///
    /// This returns Err if recursion is detected, and has no effect in this case.
    /// Recursion occurs if 'assign' is called from within the closure supplied to
    /// the 'ArcShiftCell::get'-method.
    pub fn assign(&self, other: &ArcShift<T>) -> Result<(), RecursionDetected> {
        if self.recursion.get() == 0 {
            // SAFETY:
            // Getting the inner value is safe, no other thread can be accessing it now
            *unsafe { &mut *self.inner.get() } = other.clone();
            Ok(())
        } else {
            Err(RecursionDetected)
        }
    }
    /// Reload this ArcShiftCell-instance.
    /// This allows dropping heap blocks kept alive by this instance of
    /// ArcShiftCell to be dropped.
    /// Note, this method only works when not called from within a closure
    /// supplied to the 'get' method. If such recursion occurs, this method
    /// does nothing.
    pub fn reload(&self) {
        if self.recursion.get() == 0 {
            // SAFETY:
            // For 'reload' to be safe, we must be sure that no other execution has a reference
            // to 'self.item', since the value it points to might be dropped.
            // This guarantee is fulfilled, because there is no way to get such a reference
            // other than receiving it in the callback given to 'get'. And for as long as the
            // callback is executing, 'recursion' will be != 0.
            unsafe { &mut *self.inner.get() }.reload()
        }
    }
    /// Create an ArcShift-instance pointing to the same data
    pub fn make_arcshift(&self) -> ArcShift<T> {
        // SAFETY:
        // ArcShiftCell is not Sync, and 'reload' does not recursively call into user
        // code, so we know no other operation can be ongoing.
        unsafe { &mut *self.inner.get() }.clone()
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
