use core::mem::ManuallyDrop;
use crate::deferred_panics_helper::IDropHandler;
use crate::{debug_println, IMetadata, ItemHolder};
use core::marker::PhantomData;
pub(crate) struct DropHandler<T:?Sized>(PhantomData<*mut T>);

impl<T:?Sized> Default for DropHandler<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<T:?Sized> DropHandler<T> {
    #[inline(always)]
    pub(crate) fn resume_any_panics(&self) {

    }
    #[inline]
    pub(crate) fn do_drop_value(&mut self, _value: T) where T:Sized{

    }
}
impl<T:?Sized,M:IMetadata> IDropHandler<T,M> for DropHandler<T> {
    #[cfg_attr(test, mutants::skip)]
    fn do_drop(&mut self, item_ptr: *mut ItemHolder<T, M>) {
        let payload = unsafe { &(*item_ptr).payload };
        debug_println!("payload ref created (std drop)");

        // SAFETY: item_ptr is now uniquely owned. We can drop it.
        unsafe { ManuallyDrop::drop(&mut *payload.get()) };
    }

    #[cfg_attr(test, mutants::skip)]
    fn report_sole_user(&mut self) {

    }
}

