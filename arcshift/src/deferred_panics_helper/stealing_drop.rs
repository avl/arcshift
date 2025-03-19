use crate::deferred_panics_helper::DropHandler;
use crate::deferred_panics_helper::IDropHandler;
use crate::{debug_println, ItemHolder, SizedMetadata};
use core::ptr::addr_of_mut;

/// A drop handler that will persist the most recent object, allowing
/// it to be returned to the user. All other objects will be dropped.
pub(crate) struct StealingDropHandler<T> {
    regular: DropHandler<T>,
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
        if let Some(prev) = self.stolen.take() {
            self.regular.do_drop_value(prev)
        }
        // None of the below ops can panic
        // SAFETY: item_ptr must be valid, guaranteed by caller.
        let payload = unsafe { addr_of_mut!((*item_ptr).payload).read() };
        debug_println!("payload ref created (stealing drop)");
        // item_ptr is now uniquely owned. We can safely take it..
        let payload = core::mem::ManuallyDrop::into_inner(payload.into_inner());
        self.stolen = Some(payload);
        debug_println!("payload stolen");
    }

    fn report_sole_user(&mut self) {
        self.sole_user = true;
    }
}
