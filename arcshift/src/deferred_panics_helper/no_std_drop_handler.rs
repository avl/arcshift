use crate::deferred_panics_helper::IDropHandler;
use crate::{debug_println, IMetadata, ItemHolder};

pub(crate) struct DropHandler<T: ?Sized> {
    droppers: alloc::vec::Vec<alloc::boxed::Box<T>>,
}
impl<T: ?Sized> Default for DropHandler<T> {
    #[cfg_attr(test, mutants::skip)]
    fn default() -> Self {
        Self {
            droppers: alloc::vec::Vec::new(),
        }
    }
}
impl<T: ?Sized, M: IMetadata> IDropHandler<T, M> for DropHandler<T> {
    #[cfg_attr(test, mutants::skip)]
    fn do_drop(&mut self, item_ptr: *mut ItemHolder<T, M>) {
        debug_println!("no_std do_drop called");
        let payload = unsafe { (*item_ptr).take_boxed_payload() };
        debug_println!("Scheduling drop of item");
        self.droppers.push(payload);
    }

    #[cfg_attr(test, mutants::skip)]
    fn report_sole_user(&mut self) {}
}
impl<T: ?Sized> DropHandler<T> {
    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn resume_any_panics(self) {
        let mut tself = self;
        debug_println!("Resuming all panics");
        tself.droppers.clear()
    }
    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn do_drop_value(&mut self, value: T)
    where
        T: Sized,
    {
        self.droppers.push(alloc::boxed::Box::new(value));
    }
}
