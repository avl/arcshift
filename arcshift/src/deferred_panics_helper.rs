//! This module contains routines to help ensure that panicking drop-implementations
//! do not cause corruption in the heap data-structures. The strategy to achieve
//! this differs depending on if we run in `no_std` case or not.
//! While running in `no_std`, dropping is deferred until after all lock-free memory
//! structures have been updated, at some extra cost.
//! When not using `no_std`, `catch_unwind` is used to catch panics and resume them
//! when it is safe.
use crate::{IMetadata, ItemHolder};

pub(crate) trait IDropHandler<T: ?Sized, M: IMetadata> {
    fn do_drop(&mut self, ptr: *mut ItemHolder<T, M>);
    /// Call this if
    fn report_sole_user(&mut self);
}

#[cfg(feature = "std")]
pub(crate) mod std_drop_handler;

#[cfg(all(not(feature = "nostd_unchecked_panics"), not(feature = "std")))]
pub(crate) mod no_std_drop_handler;

#[cfg(all(feature = "nostd_unchecked_panics", not(feature = "std")))]
pub(crate) mod unchecked_drop_handler;

#[cfg(feature = "std")]
pub(crate) use std_drop_handler::DropHandler;

#[cfg(all(not(feature = "nostd_unchecked_panics"), not(feature = "std")))]
pub(crate) use no_std_drop_handler::DropHandler;

#[cfg(all(feature = "nostd_unchecked_panics", not(feature = "std")))]
pub(crate) use unchecked_drop_handler::DropHandler;
pub(crate) mod stealing_drop;
