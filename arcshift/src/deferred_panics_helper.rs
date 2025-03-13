//! This module contains routines to help defer panics to outside of the critical sections
//! handling the lock free data structure. Instead, unwinding is deferred to after
//! the lock free structures have been updated. This avoids potential memory leaks, when
//! multiple objects need to be dropped simultaneously, and the first drop impl
//! panics. In this case we still wish to call other drop handlers and not resume unwind
//! until all drops have occurred.
use crate::{debug_println, IMetadata, ItemHolder, SizedMetadata};
#[cfg(feature = "std")]
use core::any::Any;
use core::mem::ManuallyDrop;
use core::ptr::addr_of_mut;
#[allow(unused)]
use core::sync::atomic::Ordering;
#[cfg(feature = "std")]
use std::panic::{resume_unwind, AssertUnwindSafe};

pub(crate) trait IDropHandler<T: ?Sized, M: IMetadata> {
    fn do_drop(&mut self, ptr: *mut ItemHolder<T, M>);
    /// Call this if
    fn report_sole_user(&mut self);
}

/// A drop handler that will persist the most recent panic, allowing it
/// to be unwinded later
#[derive(Default)]
pub(crate) struct DropHandler {
    #[cfg(feature = "std")]
    panic: Option<alloc::boxed::Box<dyn Any + Send + 'static>>,
}

/// A drop handler that will persist the most recent object, allowing
/// it to be returned to the user. All other objects will be dropped.
pub(crate) struct StealingDropHandler<T> {
    regular: DropHandler,
    stolen: Option<T>,
    sole_user: bool,
}

impl<T> Default for StealingDropHandler<T> {
    fn default() -> Self {
        Self {
            regular: Default::default(),
            stolen: None,
            sole_user: false,
        }
    }
}

impl<T> StealingDropHandler<T> {
    pub(crate) fn take_stolen(self) -> Option<T> {
        self.regular.resume_any_panics();
        self.sole_user.then_some(self.stolen?)
    }
}
impl<T> IDropHandler<T, SizedMetadata> for StealingDropHandler<T> {
    fn do_drop(&mut self, item_ptr: *mut ItemHolder<T, SizedMetadata>) {
        self.regular.run(|| {
            //This will drop any previously stolen payload.
            self.stolen.take();
        });
        // None of the below ops can panic
        // SAFETY: item_ptr must be valid, guaranteed by caller.
        let payload = unsafe { addr_of_mut!((*item_ptr).payload).read() };
        debug_println!("payload ref created");
        // SAFETY: item_ptr is now uniquely owned, and the flags of 'next' tell us it has not had
        // its payload dropped yet. We can drop it.
        let payload = ManuallyDrop::into_inner(payload.into_inner());
        self.stolen = Some(payload);
        debug_println!("payload stolen");
    }

    fn report_sole_user(&mut self) {
        self.sole_user = true;
    }
}

impl<T: ?Sized, M: IMetadata> IDropHandler<T, M> for DropHandler {
    fn do_drop(&mut self, item_ptr: *mut ItemHolder<T, M>) {
        self.run(|| {
            // SAFETY: item_ptr must be valid, guaranteed by caller.
            let payload = unsafe { &(*item_ptr).payload };
            debug_println!("payload ref created");

            // SAFETY: item_ptr is now uniquely owned, and the flags of 'next' tell us it has not had
            // its payload dropped yet. We can drop it.
            unsafe { ManuallyDrop::drop(&mut *payload.get()) };
            debug_println!("payload dropped");
        })
    }

    fn report_sole_user(&mut self) {}
}

impl DropHandler {
    pub(crate) fn run(&mut self, job: impl FnOnce()) {
        // We do use AssertUnwindSafe here.
        // Most of the time, this is perfectly fine, since this crate doesn't
        // care about the internals of the payload type (T), so any inconsistency
        // left there is fine to this crate. For the user, they won't typically
        // observe any inconsistency either, since we resume_unwind (re-throw) this panic
        // before returning to the user.
        //
        // There is one exception to this. For example, if multiple objects are dropped in the same
        // ArcShift-invocation, and the first drop panics, and the second object has a
        // reference to some third object that the first drop partially updated and left in
        // an inconsistent state.
        //
        // This is deemed acceptable. Alternatives would be :
        //
        // * Require that all payload objects implement UnwindSafe. Clearly annoying.
        // * Don't run subsequent drop-methods if the first panicked. Since leaking is safe,
        //   this is perhaps technically more idiomatic than the chosen solution. However,
        //   Drop-impls should rarely need to update foreign objects, and also, drop-methods
        //   really shouldn't panic at all.

        #[cfg(feature = "std")]
        {
            match std::panic::catch_unwind(AssertUnwindSafe(job)) {
                Ok(()) => {}
                Err(err) => {
                    self.panic.get_or_insert(err);
                }
            }
        }
        #[cfg(not(feature = "std"))]
        {
            job();
        }
    }
    pub(crate) fn resume_any_panics(self) {
        #[cfg(feature = "std")]
        if let Some(panic) = self.panic {
            resume_unwind(panic);
        }
    }
}
