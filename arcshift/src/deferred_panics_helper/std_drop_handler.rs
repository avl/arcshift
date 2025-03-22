use crate::deferred_panics_helper::IDropHandler;
use crate::{debug_println, IMetadata, ItemHolder};
use core::any::Any;
use core::mem::ManuallyDrop;
use core::marker::PhantomData;

/// A drop handler that will persist the most recent panic, allowing it
/// to be unwinded later
pub(crate) struct DropHandler<T: ?Sized> {
    panic: Option<alloc::boxed::Box<dyn Any + Send + 'static>>,
    phantom: PhantomData<*mut T>,
}

impl<T: ?Sized> Default for DropHandler<T> {
    fn default() -> Self {
        Self {
            panic: None,
            phantom: Default::default(),
        }
    }
}

impl<T: ?Sized, M: IMetadata> IDropHandler<T, M> for DropHandler<T> {
    fn do_drop(&mut self, item_ptr: *mut ItemHolder<T, M>) {
        self.run(|| {
            // SAFETY: item_ptr must be valid, guaranteed by caller.
            let payload = unsafe { &(*item_ptr).payload };
            debug_println!("payload ref created (std drop)");

            // SAFETY: item_ptr is now uniquely owned. We can drop it.
            unsafe { ManuallyDrop::drop(&mut *payload.get()) };
            debug_println!("payload dropped");
        })
    }

    fn report_sole_user(&mut self) {}
}

impl<T: ?Sized> DropHandler<T> {
    pub(crate) fn do_drop_value(&mut self, value: T)
    where
        T: Sized,
    {
        self.run(move || {
            let _ = value;
        });
    }
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

        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)) {
            Ok(()) => {}
            Err(err) => {
                self.panic.get_or_insert(err);
            }
        }
    }
    pub(crate) fn resume_any_panics(self) {
        if let Some(panic) = self.panic {
            std::panic::resume_unwind(panic);
        }
    }
}
