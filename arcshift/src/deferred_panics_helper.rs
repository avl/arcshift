//! This module contains routines to help defer panics to outside of the critical sections
//! handling the lock free data structure. Instead, unwinding is deferred to after
//! the lock free structures have been updated. This avoids potential memory leaks, when
//! multiple objects need to be dropped simultaneously, and the first drop impl
//! panics. In this case we still wish to call other drop handlers and not resume unwind
//! until all drops have occurred.
use crate::{IMetadata, ItemHolder};

pub(crate) trait IDropHandler<T: ?Sized, M: IMetadata> {
    fn do_drop(&mut self, ptr: *mut ItemHolder<T, M>);
    /// Call this if
    fn report_sole_user(&mut self);
}

#[cfg(feature = "std")]
pub(crate) mod std_drop_handler;

#[cfg(not(feature = "std"))]
pub(crate) mod no_std_drop_handler;

#[cfg(feature = "std")]
pub(crate) use std_drop_handler::DropHandler;

#[cfg(not(feature = "std"))]
pub(crate) use no_std_drop_handler::DropHandler;

pub(crate) mod stealing_drop;
