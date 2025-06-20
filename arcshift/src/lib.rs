#![no_std]
#![cfg_attr(feature = "nightly", feature(ptr_metadata))]
#![deny(warnings)]
#![forbid(clippy::undocumented_unsafe_blocks)]
#![deny(missing_docs)]

//! # Introduction to ArcShift
//!
//! [`ArcShift`] is a data type similar to `std::sync::Arc`, except that it allows updating
//! the value pointed to. It can be used as a replacement for
//! `std::sync::Arc<std::sync::RwLock<T>>`, giving much faster read access.
//!
//! Updating the value in ArcShift is significantly more expensive than writing `std::sync::RwLock`,
//! so ArcShift is most suited to cases where updates are infrequent.
//!
//! ## Example
//! ```
//! # #[cfg(any(feature="loom",feature="shuttle"))]
//! # pub fn main() {}
//! # #[cfg(not(any(feature="loom",feature="shuttle")))]
//! # pub fn main() {
//! # extern crate arcshift;
//! # use arcshift::ArcShift;
//! use std::thread;
//!
//!
//! let mut arc = ArcShift::new("Hello".to_string());
//! let mut arc2 = arc.clone();
//!
//!
//! let j1 = thread::spawn(move||{
//!     println!("Value in thread 1: '{}'", arc.get()); //Prints 'Hello'
//!     arc.update("New value".to_string());
//!     println!("Updated value in thread 1: '{}'", arc.get()); //Prints 'New value'
//! });
//!
//! let j2 = thread::spawn(move||{
//!     // Prints either 'Hello' or 'New value', depending on scheduling:
//!     println!("Value in thread 2: '{}'", arc2.get());
//! });
//!
//! j1.join().unwrap();
//! j2.join().unwrap();
//! # }
//! ```
//!
//! # Strong points
//! * Easy to use (similar to Arc)
//! * Extensively tested
//! * All functions are lock free ( <https://en.wikipedia.org/wiki/Non-blocking_algorithm> )
//! * For use cases where no updates occur, performance is very good (much
//!   better than RwLock or Mutex).
//! * Updates are reasonably fast (think, 15-100 nanoseconds), but much slower than Mutex- or
//!   RwLock-writes.
//! * The function [`ArcShift::shared_non_reloading_get`] allows access without any overhead
//!   compared to regular Arc (benchmarks show identical performance to Arc).
//! * ArcShift does not rely on thread-local variables.
//! * Supports unsized types (i.e, you can use `ArcShift<[u8]>`)
//! * ArcShift is no_std compatible (though 'alloc' is required, since ArcShift is a heap
//!   data structure). Compile with "default-features=false" to enable no_std
//!   compatibility.
//!
//! # Limitations
//!
//! ArcShift achieves its performance at the expense of the following disadvantages:
//!
//! * ArcShift's performance relies on being able to update its pointer when
//!   new values are detected. This means that ArcShift is most efficient when each
//!   thread has a mutable ArcShift instance. This can often be achieved by cloning the ArcShift,
//!   and distributing one owned copy to every thread (these clones all point to the same
//!   inner value). ArcShift can still be used with only shared access ([`ArcShift::shared_get`]),
//!   and performance is still very good as long as the pointer is current. However, if
//!   the ArcShift instance is stale (needs reloading, because an update has occurred), reads will
//!   be approximately twice as costly as for RwLock. One mitigation for this is to use
//!   [`cell::ArcShiftCell`]. However, this type is not `Sync`, only `Send`.
//! * When modifying the value, the old version of the value lingers in memory until
//!   the last ArcShift that uses it has reloaded. Such a reload only happens when the ArcShift
//!   is accessed using a unique (`&mut`) access (like [`ArcShift::get`] or [`ArcShift::reload`]).
//!   This can be partially mitigated by using the [`ArcShiftWeak`]-type for long-lived
//!   never-reloaded instances.
//! * Modifying the value is approximately 10x more expensive than modifying an `Arc<Mutex<T>>`.
//!   That said, if you're storing anything significantly more complex than an integer, the overhead
//!   of ArcShift may be insignificant.
//! * When the value is modified, the next subsequent reload is slower than an `Arc<RwLock<T>>`
//!   access.
//! * ArcShift is its own datatype. It is in no way compatible with `Arc<T>`.
//! * At most usize::MAX/8 instances of ArcShift or ArcShiftWeak can be created for each value.
//!   (this is because it uses some bits of its weak refcount to store metadata).
//!
//! # Detailed performance characteristics
//!
//!
//! * [`ArcShift::get`] - Very good average performance.
//!   Checking for new values requires a single atomic operation, of the least expensive kind
//!   (Ordering::Relaxed). On x86_64, this is the exact same machine operation as a regular
//!   memory access, and also on arm it is not an expensive operation. The cost of such access
//!   is much smaller than a mutex access, even an uncontended one. In the case where a reload
//!   is actually necessary, there is a significant performance impact (but still typically
//!   below 150ns for modern machines (2025)).
//!
//!   If other instances have made updates, subsequent accesses will have a penalty. This
//!   penalty can be significant, because previous values may have to be dropped.  However,
//!   when any updates have been processed, subsequent accesses will be fast again. It is
//!   guaranteed that any update that completed before the execution of [`ArcShift::get`]
//!   started, will be visible.
//!
//! * [`ArcShift::shared_get`] - Good performance as long as the value is not stale.
//!   If self points to a previous value, each call to `shared_get` will traverse the
//!   memory structures to find the most recent value.
//!
//!   There are three cases:
//!   * The value is up-to-date. In this case, execution is very fast.
//!   * The value is stale, but no write is in progress. Expect a penalty equal to
//!     twice the cost of an RwLock write, approximately.
//!   * The value is stale, and there is a write in progress. This is a rare race condition.
//!     Expect a severe performance penalty (~10-20x the cost of an RwLock write).
//!
//!   `shared_get` also guarantees that any updates that completed before it was called,
//!   will be visible.
//!
//! * [`ArcShift::shared_non_reloading_get`] - No overhead compared to plain Arc.
//!   Will not reload, even if the ArcShift instance is stale. May thus return an old
//!   value. If shared_get has been used previously, this method may return an older
//!   value than what the shared_get returned.
//!
//! * [`ArcShift::reload`] - Similar cost to [`ArcShift::get`].
//!
//! * [`ArcShift::clone`] - Fast. Requires a single atomic increment, and an atomic read.
//!   If the current instance is stale, the cloned value will be reloaded, with identical
//!   cost to [`ArcShift::get`].
//!
//! * Drop - Can be slow. The last remaining owner of a value will drop said value.
//!
//!
//! # Motivation
//!
//! The primary raison d'être for [`ArcShift`] is to be a version of Arc which allows
//! updating the stored value, with very little overhead over regular Arc for read heavy
//! loads.
//!
//! One such motivating use-case for ArcShift is hot-reloadable assets in computer games.
//! During normal usage, assets do not change. All benchmarks and play experience will
//! be dependent only on this baseline performance. Ideally, we therefore want to have
//! a very small performance penalty for the case when assets are *not* updated, comparable
//! to using regular `std::sync::Arc`.
//!
//! During game development, artists may update assets, and hot-reload is a very
//! time-saving feature. A performance hit during asset-reload is acceptable though.
//! ArcShift prioritizes base performance, while accepting a penalty when updates are made.
//!
//! ArcShift can, of course, be useful in other domains than computer games.
//!
//!
//! # Panicking drop methods
//! If a drop implementation panics, ArcShift will make sure that the internal data structures
//! remain uncorrupted. When run without the std-library, some memory leakage will occur every
//! time a drop method panics. With the std-library, only memory owned by the payload type
//! might leak.
//!
//! # No_std
//! By default, arcshift uses the rust standard library. This is enabled by the 'std' feature, which
//! is enabled by default. ArcShift can work without the full rust std library. However, this
//! comes at a slight performance cost. When the 'std' feature is enabled (which it is by default),
//! `catch_unwind` is used to guard drop functions, to make sure memory structures are not corrupted
//! if a user-supplied drop method panics. However, to ensure the same guarantee when running
//! without std, arcshift presently moves allocations to temporary boxes to be able to run drop
//! after all memory traversal is finished. This requires multiple allocations, which makes
//! operation without 'std' slower. Panicking drop methods can also lead to memory leaks
//! without the std. The memory structures remain intact, and no undefined behavior occurs.
//! The performance penalty is only present during updates.
//!
//! If the overhead mentioned in the previous paragraph is unacceptable, and if the final binary
//! is compiled with panic=abort, this extra cost can be mitigated. Enable the feature
//! "nostd_unchecked_panics" to do this. This must never be done if the process will ever continue
//! executing after a panic, since it can lead to memory reclamation essentially being disabled
//! for any ArcShift-chain that has had a panicking drop. However, no UB will result, in any case.
//!
//! # Implementation
//!
//! The basic idea of ArcShift is that each ArcShift instance points to a small heap block,
//! that contains the pointee value of type T, three reference counts, and 'prev'/'next'-pointers.
//! The 'next'-pointer starts out as null, but when the value in an ArcShift is updated, the
//! 'next'-pointer is set to point to the updated value.
//!
//! This means that each ArcShift-instance always points at a valid value of type T. No locking
//! or synchronization is required to get at this value. This is why ArcShift instances are fast
//! to use. There is the drawback that as long as an ArcShift-instance exists, whatever value
//! it points to must be kept alive. Each time an ArcShift instance is accessed mutably, we have
//! an opportunity to update its pointer to the 'next' value. The operation to update the pointer
//! is called a 'reload'.
//!
//! When the last ArcShift-instance releases a particular value, it will be dropped.
//!
//! ArcShiftWeak-instances also keep pointers to the heap blocks mentioned above, but the value T
//! in the block can be dropped while being held by an ArcShiftWeak. This means that an ArcShiftWeak-
//! instance only consumes `std::mem::size_of::<T>()` bytes plus 5 words of memory, when the value
//! it points to has been dropped. When the ArcShiftWeak-instance is reloaded, or dropped, that
//! memory is also released.
//!
//! # Prior Art
//!
//! ArcShift is very much inspired by arc-swap. The two crates can be used for similar problems.
//! They have slightly different APIs, one or the other may be a more natural fit depending on
//! the problem. ArcShift may be faster for some problems, slower for others.
//!
//!
//! # Pitfall #1 - lingering memory usage
//!
//! Be aware that ArcShift instances that are just "lying around" without ever being reloaded,
//! will keep old values around, taking up memory. This is a fundamental drawback of the approach
//! taken by ArcShift. One workaround is to replace any long-lived infrequently reloaded instances of
//! [`ArcShift`] with [`ArcShiftWeak`]. This alleviates the problem, though heap storage of approx
//! `size_of<T>` + 5 words is still expended.
//!
//! # Pitfall #2 - reference count limitations
//!
//! ArcShift uses usize data type for the reference counts. However, it reserves two bits for
//! tracking some metadata. This leaves usize::MAX/4 as the maximum usable reference count. To
//! avoid having to check the refcount twice (once before increasing the count), we set the limit
//! at usize::MAX/8, and check the count after the atomic operation. This has the effect that if more
//! than usize::MAX/8 threads clone the same ArcShift instance concurrently, the unsoundness will occur.
//! However, this is considered acceptable, because this exceeds the possible number
//! of concurrent threads by a huge safety margin. Also note that usize::MAX/8 ArcShift instances would
//! take up usize::MAX bytes of memory, which is very much impossible in practice. By leaking
//! ArcShift instances in a tight loop it is still possible to achieve a weak count of usize::MAX/8,
//! in which case ArcShift will panic.
//!
//! # A larger example
//!
//! ```no_run
//! # extern crate arcshift;
//! # use arcshift::ArcShift;
//!
//! struct CharacterModel {
//!     /* 3D model, textures, etc*/
//! }
//!
//! struct World {
//!     models: Vec<ArcShift<CharacterModel>>
//! }
//!
//! /// Loads models. Regularly scans filesystem,
//! /// updates models when their files change on disk.
//! fn load_models() -> Vec<ArcShift<CharacterModel>> {
//!     let models: Vec<ArcShift<CharacterModel>> = vec![];
//!
//!     /* Somehow load models */
//!
//!     let mut models_for_reloader = models.clone();
//!     std::thread::spawn(move||{
//!         loop {
//!             /* detect file system changes*/
//!             let changed_model = 0usize;
//!
//!             models_for_reloader[changed_model].update(CharacterModel{/* newly loaded*/});
//!         }
//!
//!     });
//!
//!     models
//! }
//!
//! fn run_game() {
//!     let mut world = World {
//!         models: load_models()
//!     };
//!     loop {
//!         run_game_logic(&mut world);
//!     }
//! }
//!
//! fn run_game_logic(world: &mut World) {
//!     /*
//!         Do game logic, possibly in multiple threads, accessing different parts of World,
//!         possibly cloning 'ArcShift' instances for use by other threads
//!     */
//!
//!     for model in world.models.iter_mut() {
//!         // Accessing ArcShift using 'get' ensures
//!         // old versions do not linger in RAM.
//!         let model_ref : &CharacterModel = model.get();
//!         // Do stuff with 'model_ref'
//!     }
//! }
//!
//!
//! ```
//!

#[cfg(feature = "std")]
extern crate std;

extern crate alloc;

use alloc::boxed::Box;

use core::alloc::Layout;

use crate::deferred_panics_helper::DropHandler;
use crate::deferred_panics_helper::IDropHandler;
use core::cell::UnsafeCell;
use core::fmt::{Debug, Formatter};
use core::marker::PhantomData;
#[allow(unused)]
use core::mem::align_of_val;
use core::mem::size_of;
#[allow(unused)]
use core::mem::size_of_val;
use core::mem::{transmute, ManuallyDrop};
use core::panic::UnwindSafe;
use core::ptr::{addr_of_mut, null, null_mut, NonNull};
use core::sync::atomic::Ordering;
// About unsafe code in this crate:
// Some private functions contain unsafe code, and place limitations on their
// callers, without these private functions being marked unsafe.
// The rationale is that there are lots of operations that simply aren't unsafe, like
// assigning null to a pointer, that could cause UB in unsafe code in this crate.
// This crate is inherently dependent on all the code in it being correct. Therefore,
// marking more functions unsafe buys us very little.
// Note! The API of this crate is 100% safe and UB should be impossible to trigger through the API.
// All public methods are 100% sound, this argument only concerns private methods.

// All atomic primitives are reexported from a
// local module called 'atomic', so we can easily change between using
// types from 'std' (normal case) and types from shuttle/loom testing libraries.

mod deferred_panics_helper;

/// Declarations of atomic ops for using Arcshift in production
#[cfg(all(not(loom), not(feature = "shuttle")))]
mod atomic {
    pub use core::sync::atomic::{fence, AtomicPtr, AtomicUsize, Ordering};
    #[allow(unused)]
    #[cfg(feature = "std")]
    pub use std::sync::{Arc, Mutex};
    #[allow(unused)]
    #[cfg(feature = "std")]
    pub use std::thread;
}

/// Declarations for verifying Arcshift using 'shuttle'
#[cfg(feature = "shuttle")]
mod atomic {
    #[allow(unused)]
    pub use shuttle::sync::atomic::{fence, AtomicPtr, AtomicUsize, Ordering};
    #[allow(unused)]
    pub use shuttle::sync::{Arc, Mutex};
    #[allow(unused)]
    pub use shuttle::thread;
}

/// Declarations for verifying Arcshift using 'loom'
#[cfg(loom)]
mod atomic {
    pub use loom::sync::atomic::{fence, AtomicPtr, AtomicUsize, Ordering};
    #[allow(unused)]
    pub use loom::sync::{Arc, Mutex};
    #[allow(unused)]
    pub use loom::thread;
}

#[doc(hidden)]
#[cfg(all(feature = "std", feature = "debug", not(loom)))]
#[macro_export]
macro_rules! debug_println {
    ($($x:tt)*) => {
        std::println!("{:?}: {}", $crate::atomic::thread::current().id(), std::format!($($x)*))
    }
}

#[doc(hidden)]
#[cfg(all(feature = "std", feature = "debug", loom))]
#[macro_export]
macro_rules! debug_println {
    ($($x:tt)*) => { std::println!($($x)*) }
}

#[doc(hidden)]
#[cfg(any(not(feature = "std"), not(feature = "debug")))]
#[macro_export]
macro_rules! debug_println {
    ($($x:tt)*) => {{}};
}

/// Smart pointer with similar use case as std::sync::Arc, but with
/// the added ability to atomically replace the contents of the Arc.
/// See `crate` documentation for more information.
///
/// ```rust
/// # extern crate arcshift;
/// # #[cfg(any(feature="loom",feature="shuttle"))]
/// # fn main() {}
/// # #[cfg(not(any(feature="loom",feature="shuttle")))]
/// # pub fn main() {
/// # use arcshift::ArcShift;
/// let mut instance = ArcShift::new("test");
/// println!("Value: {:?}", instance.get());
/// # }
/// ```
pub struct ArcShift<T: ?Sized> {
    item: NonNull<ItemHolderDummy<T>>,

    // Make sure ArcShift is invariant.
    // ArcShift instances with different lifetimes of T should not be compatible,
    // since could lead to a short-lived value T being insert into a chain which
    // also has long-lived ArcShift instances with long-lived T.
    pd: PhantomData<*mut T>,
}
impl<T> UnwindSafe for ArcShift<T> {}

impl<T: ?Sized> Debug for ArcShift<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "ArcShift({:?})", self.shared_non_reloading_get())
    }
}

impl<T: Default> Default for ArcShift<T> {
    fn default() -> Self {
        ArcShift::new(Default::default())
    }
}

impl<T: ?Sized> Debug for ArcShiftWeak<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "ArcShiftWeak(..)")
    }
}

/// ArcShiftWeak is a way to keep a pointer to an object without preventing said object
/// from being deallocated. This can be useful when creating cyclic data structure, to avoid
/// memory leaks.
///
/// ```no_run
/// # use arcshift::ArcShift;
/// # #[cfg(any(feature="loom",feature="shuttle"))]
/// # fn main() {}
/// # #[cfg(not(any(feature="loom",feature="shuttle")))]
/// # fn main() {
/// # extern crate arcshift;
/// # use arcshift::ArcShiftWeak;
/// let original_instance = ArcShift::new("test");
/// let light_instance = ArcShift::downgrade(&original_instance);
/// let mut instance = light_instance.upgrade().unwrap();
/// println!("Value: {:?}", instance.get());
/// # }
/// ```
///
/// WARNING! Because of implementation reasons, each instance of ArcShiftWeak will claim
/// a memory equal to `size_of::<T>` (plus 5 words), even if the value inside it has freed,
/// and even if all other instances of ArcShift/ArcShiftWeak have been dropped.
/// If this limitation is unacceptable, consider using `ArcShiftWeak<Box<T>>` as your datatype,
/// or possibly using a different crate.
pub struct ArcShiftWeak<T: ?Sized> {
    item: NonNull<ItemHolderDummy<T>>,
}

/// Module with a convenient cell-like data structure for reloading ArcShift instances
/// despite only having shared access.
pub mod cell;

#[inline(always)]
const fn is_sized<T: ?Sized>() -> bool {
    size_of::<&T>() == size_of::<&()>()
}

/// SAFETY:
/// If `T` is `Sync` and `Send`, `ArcShift<T>` can also be `Sync`
unsafe impl<T: Sync + Send + ?Sized> Sync for ArcShift<T> {}

/// SAFETY:
/// If `T` is `Sync` and `Send`, `ArcShift<T>` can also be `Send`
unsafe impl<T: Sync + Send + ?Sized> Send for ArcShift<T> {}

/// SAFETY:
/// If `T` is `Sync` and `Send`, `ArcShiftWeak<T>` can also be `Sync`
unsafe impl<T: Sync + Send + ?Sized> Sync for ArcShiftWeak<T> {}

/// SAFETY:
/// If `T` is `Sync` and `Send`, `ArcShiftWeak<T>` can also be `Send`
unsafe impl<T: Sync + Send + ?Sized> Send for ArcShiftWeak<T> {}

#[repr(transparent)]
struct UnsizedMetadata<T: ?Sized> {
    #[cfg(feature = "nightly")]
    meta: <T as std::ptr::Pointee>::Metadata,
    #[cfg(not(feature = "nightly"))]
    meta: *const (),
    phantom: PhantomData<T>,
}
impl<T: ?Sized> Clone for UnsizedMetadata<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: ?Sized> Copy for UnsizedMetadata<T> {}
impl<T: ?Sized> core::fmt::Debug for UnsizedMetadata<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Metadata({:?})", self.meta)
    }
}

#[allow(dead_code)]
#[repr(C)]
struct FatPtr<T: ?Sized> {
    ptr: *mut u8,
    meta: UnsizedMetadata<T>,
}

#[inline]
fn arc_from_raw_parts_mut<T: ?Sized, M: IMetadata>(
    data_ptr: *mut u8,
    metadata: UnsizedMetadata<T>,
) -> *mut ItemHolder<T, M> {
    // SAFETY:
    // This is the best I managed without using nightly-only features (August 2024).
    // It is sound as long as the actual internal representation of fat pointers doesn't change.
    #[cfg(not(feature = "nightly"))]
    unsafe {
        core::mem::transmute_copy(&FatPtr {
            ptr: data_ptr,
            meta: metadata,
        })
    }
    #[cfg(feature = "nightly")]
    {
        std::ptr::from_raw_parts_mut(data_ptr, metadata.meta)
    }
}

#[inline]
#[cfg(not(any(feature = "std", feature = "nostd_unchecked_panics")))]
#[cfg_attr(test, mutants::skip)]
pub(crate) fn ptr_from_raw_parts_mut<T: ?Sized>(
    data_ptr: *mut u8,
    metadata: UnsizedMetadata<T>,
) -> *mut T {
    // SAFETY:
    // This is the best I managed without using nightly-only features (August 2024).
    // It is sound as long as the actual internal representation of fat pointers doesn't change.
    #[cfg(not(feature = "nightly"))]
    unsafe {
        core::mem::transmute_copy(&FatPtr {
            ptr: data_ptr,
            meta: metadata,
        })
    }
    #[cfg(feature = "nightly")]
    {
        std::ptr::from_raw_parts_mut(data_ptr, metadata.meta)
    }
}

impl<T: ?Sized> UnsizedMetadata<T> {
    #[inline]
    #[cfg(not(feature = "nightly"))]
    fn polyfill_metadata(cur_ptr: *const T) -> *const () {
        if is_sized::<T>() {
            unreachable!() //We're never using this function for sized data
        } else {
            let ptrptr = &cur_ptr as *const *const T as *const *const ();
            // SAFETY:
            // This is a trick to get at the 'metadata'-part of the fat-ptr 'cur_ptr'.
            // It works in practice, as long as the internal representation of fat pointers doesn't
            // change.
            unsafe { *ptrptr.wrapping_offset(1) }
        }
    }

    #[inline]
    pub fn new(cur_ptr: *const T) -> UnsizedMetadata<T> {
        UnsizedMetadata {
            #[cfg(feature = "nightly")]
            meta: std::ptr::metadata(cur_ptr),
            #[cfg(not(feature = "nightly"))]
            meta: Self::polyfill_metadata(cur_ptr),
            phantom: PhantomData,
        }
    }
}

fn get_holder_layout<T: ?Sized>(ptr: *const T) -> Layout {
    // SAFETY:
    // The pointer 'ptr' is a valid pointer
    let payload_layout = Layout::for_value(unsafe { &*ptr });
    if is_sized::<T>() {
        let layout = Layout::new::<ItemHolder<(), SizedMetadata>>();
        let (layout, _) = layout.extend(payload_layout).unwrap();
        layout.pad_to_align()
    } else {
        let layout = Layout::new::<UnsizedMetadata<T>>();
        let (layout, _) = layout
            .extend(Layout::new::<ItemHolder<(), UnsizedMetadata<T>>>())
            .unwrap();
        let (layout, _) = layout.extend(payload_layout).unwrap();
        layout.pad_to_align()
    }
}

#[inline(always)]
fn to_dummy<T: ?Sized, M: IMetadata>(ptr: *const ItemHolder<T, M>) -> *const ItemHolderDummy<T> {
    ptr as *mut ItemHolderDummy<T>
}

#[inline(always)]
fn from_dummy<T: ?Sized, M: IMetadata>(ptr: *const ItemHolderDummy<T>) -> *const ItemHolder<T, M> {
    get_full_ptr_raw::<T, M>(ptr)
}

macro_rules! with_holder {
    ($p: expr, $t: ty, $item: ident, $f:expr) => {
        if is_sized::<$t>() {
            let $item = from_dummy::<$t, SizedMetadata>($p.as_ptr());
            $f
        } else {
            let $item = from_dummy::<$t, UnsizedMetadata<$t>>($p.as_ptr());
            $f
        }
    };
}

fn make_sized_or_unsized_holder_from_box<T: ?Sized>(
    item: Box<T>,
    prev: *mut ItemHolderDummy<T>,
) -> *const ItemHolderDummy<T> {
    if is_sized::<T>() {
        to_dummy(make_sized_holder_from_box(item, prev))
    } else {
        to_dummy(make_unsized_holder_from_box(item, prev))
    }
}

#[allow(clippy::let_and_return)]
fn get_weak_count(count: usize) -> usize {
    let count = count & ((1 << (usize::BITS - 2)) - 1);
    #[cfg(feature = "validate")]
    assert!(count < MAX_REF_COUNT / 2);
    count
}

/// Node has next, or will soon have next. Note! It may seem this is unnecessary,
/// since if there's a "next", then that next node will hold a weak ref, so a lonely
/// reference could detect that there was a 'next' anyway.
///
/// Consider the following sequence:
/// * Instance A and B point to Node N1
/// * A starts dropping. It loads weak count '2'.
/// * B updates, creating N2, and advanced to N2, and is then dropped.
/// * Weak count of N1 is now still 2, so the compare_exchange for A *succeeds*
///
/// Having this flag makes it impossible for N1 to succeed
const WEAK_HAVE_NEXT: usize = 1 << (usize::BITS - 1);
const WEAK_HAVE_PREV: usize = 1 << (usize::BITS - 2);

/// To avoid potential unsafety if refcounts overflow and wrap around,
/// we have a maximum limit for ref count values. This limit is set with a large margin relative
/// to the actual wraparound value. Note that this
/// limit is enforced post fact, meaning that if there are more than usize::MAX/8 or so
/// simultaneous threads, unsoundness can occur. This is deemed acceptable, because it is
/// impossible to achieve in practice. Especially since it basically requires an extremely long
/// running program with memory leaks, or leaking memory very fast in a tight loop. Without leaking,
/// is impractical to achieve such high refcounts, since having usize::MAX/8 ArcShift instances
/// alive on a 64-bit machine is impossible, since this would require usize::MAX/2 bytes of memory,
/// orders of magnitude larger than any existing machine (in 2025).
const MAX_REF_COUNT: usize = usize::MAX / 8;

fn get_weak_prev(count: usize) -> bool {
    (count & WEAK_HAVE_PREV) != 0
}

#[allow(unused)]
fn get_weak_next(count: usize) -> bool {
    (count & WEAK_HAVE_NEXT) != 0
}

#[cfg_attr(test, mutants::skip)]
fn initial_weak_count<T: ?Sized>(prev: *const ItemHolderDummy<T>) -> usize {
    if prev.is_null() {
        1
    } else {
        1 | WEAK_HAVE_PREV
    }
}

#[allow(unused)]
#[cfg(all(feature = "std", feature = "debug"))]
#[cfg_attr(test, mutants::skip)]
fn format_weak(weak: usize) -> std::string::String {
    let count = weak & ((1 << (usize::BITS - 2)) - 1);
    let have_next = (weak & WEAK_HAVE_NEXT) != 0;
    let have_prev = (weak & WEAK_HAVE_PREV) != 0;
    std::format!("(weak: {} prev: {} next: {})", count, have_prev, have_next)
}

#[allow(unused)]
fn make_unsized_holder_from_box<T: ?Sized>(
    item: Box<T>,
    prev: *mut ItemHolderDummy<T>,
) -> *const ItemHolder<T, UnsizedMetadata<T>> {
    let cur_ptr = Box::into_raw(item);

    debug_println!("thesize: {:?}", cur_ptr);
    let item_holder_ptr: *mut ItemHolder<T, UnsizedMetadata<T>>;

    // SAFETY:
    // The pointer 'cur_ptr' is a valid pointer (it's just been created by 'Box::into_raw'
    let payload_layout = Layout::for_value(unsafe { &*cur_ptr });
    let the_size = payload_layout.size();

    if is_sized::<T>() {
        unreachable!()
    } else {
        let layout = get_holder_layout::<T>(cur_ptr);

        let metadata = UnsizedMetadata::new(cur_ptr);
        debug_println!("Layout: {:?}, meta: {:?}", layout, metadata);
        item_holder_ptr =
            // SAFETY:
            // core::alloc::alloc requires the allocated layout to have a nonzero size. This
            // is fulfilled, since ItemHolder is non-zero sized even if T is zero-sized.
            // The returned memory is uninitialized, but we will initialize the required parts of it
            // below.
            unsafe { arc_from_raw_parts_mut(alloc::alloc::alloc(layout) as *mut _, metadata) };
        debug_println!("Sized result ptr: {:?}", item_holder_ptr);
        // SAFETY:
        // result_ptr is a valid pointer
        unsafe {
            addr_of_mut!((*item_holder_ptr).the_meta).write(metadata);
        }
    }

    // SAFETY:
    // The copy is safe because MaybeUninit<ItemHolder<T>> is guaranteed to have the same
    // memory layout as ItemHolder<T>, so we're just copying a value of T to a new location.
    unsafe {
        (addr_of_mut!((*item_holder_ptr).payload) as *mut u8)
            .copy_from(cur_ptr as *mut u8, the_size);
    }
    // SAFETY:
    // next is just an AtomicPtr-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).prev).write(atomic::AtomicPtr::new(prev as *mut _));
    }
    // SAFETY:
    // refcount is just an AtomicPtr-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).next).write(atomic::AtomicPtr::default());
    }
    // SAFETY:
    // strong_count is just an AtomicUsize-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).strong_count).write(atomic::AtomicUsize::new(1));
    }
    // SAFETY:
    // weak_count is just an AtomicUsize-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).weak_count)
            .write(atomic::AtomicUsize::new(initial_weak_count(prev)));
    }
    // SAFETY:
    // advance_count is just an AtomicUsize-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).advance_count).write(atomic::AtomicUsize::new(0));
    }

    // SAFETY:
    // result_ptr is a valid pointer
    #[cfg(feature = "validate")]
    unsafe {
        addr_of_mut!((*item_holder_ptr).magic1)
            .write(std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111));
    }

    // SAFETY:
    // result_ptr is a valid pointer
    #[cfg(feature = "validate")]
    unsafe {
        addr_of_mut!((*item_holder_ptr).magic2)
            .write(std::sync::atomic::AtomicU64::new(0x1234123412348111));
    }

    // SAFETY:
    // input_ptr is a *mut T that has been created from a Box<T>.
    // Converting it to a Box<MaybeUninit<T>> is safe, and will make sure that any drop-method
    // of T is not run. We must not drop T here, since we've moved it to the 'result_ptr'.
    let _t: Box<ManuallyDrop<T>> = unsafe { Box::from_raw(cur_ptr as *mut ManuallyDrop<T>) }; //Free the memory, but don't drop 'input'

    item_holder_ptr
}

#[allow(unused)]
fn make_sized_holder<T>(
    payload: T,
    prev: *const ItemHolderDummy<T>,
) -> *mut ItemHolder<T, SizedMetadata> {
    let holder = ItemHolder {
        the_meta: SizedMetadata,
        #[cfg(feature = "validate")]
        magic1: std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111),
        next: Default::default(),
        prev: atomic::AtomicPtr::new(prev as *mut _),
        strong_count: atomic::AtomicUsize::new(1),
        weak_count: atomic::AtomicUsize::new(initial_weak_count::<T>(prev)),
        advance_count: atomic::AtomicUsize::new(0),
        #[cfg(feature = "validate")]
        magic2: std::sync::atomic::AtomicU64::new(0x1234123412348111),
        payload: UnsafeCell::new(ManuallyDrop::new(payload)),
    };

    Box::into_raw(Box::new(holder))
}

#[allow(unused)]
fn make_sized_holder_from_box<T: ?Sized>(
    item: Box<T>,
    prev: *mut ItemHolderDummy<T>,
) -> *mut ItemHolder<T, SizedMetadata> {
    let cur_ptr = Box::into_raw(item);

    debug_println!("thesize: {:?}", cur_ptr);

    // SAFETY:
    // 'cur_ptr' is a valid pointer, it's just been created by Box::into_raw
    let payload_layout = Layout::for_value(unsafe { &*cur_ptr });
    let the_size = payload_layout.size();

    let layout = get_holder_layout::<T>(cur_ptr);

    // SAFETY:
    // The type '*mut ItemHolder<T, NoMeta>' is not actually a fat pointer. But since T:?Sized,
    // the compiler treats it like a fat pointer with a zero-size metadata, which is not
    // the exact same thing as a thin pointer (is my guess).
    // Using transmute_copy to trim off the metadata is sound.
    let item_holder_ptr: *mut ItemHolder<T, SizedMetadata> =
        unsafe { core::mem::transmute_copy(&alloc::alloc::alloc(layout)) };

    // SAFETY:
    // The copy is safe because MaybeUninit<ItemHolder<T>> is guaranteed to have the same
    // memory layout as ItemHolder<T>, so we're just copying a value of T to a new location.
    unsafe {
        (addr_of_mut!((*item_holder_ptr).payload) as *mut u8)
            .copy_from(cur_ptr as *mut u8, the_size);
    }
    // SAFETY:
    // next is just an AtomicPtr-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).prev).write(atomic::AtomicPtr::new(prev as *mut _));
    }
    // SAFETY:
    // refcount is just an AtomicPtr-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).next).write(atomic::AtomicPtr::default());
    }
    // SAFETY:
    // strong_count is just an AtomicUsize-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).strong_count).write(atomic::AtomicUsize::new(1));
    }
    // SAFETY:
    // weak_count is just an AtomicUsize-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).weak_count)
            .write(atomic::AtomicUsize::new(initial_weak_count(prev)));
    }
    // SAFETY:
    // advance_count is just an AtomicUsize-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).advance_count).write(atomic::AtomicUsize::new(0));
    }

    // SAFETY:
    // result_ptr is a valid pointer
    #[cfg(feature = "validate")]
    unsafe {
        addr_of_mut!((*item_holder_ptr).magic1)
            .write(std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111));
    }

    // SAFETY:
    // result_ptr is a valid pointer
    #[cfg(feature = "validate")]
    unsafe {
        addr_of_mut!((*item_holder_ptr).magic2)
            .write(std::sync::atomic::AtomicU64::new(0x1234123412348111));
    }

    // SAFETY:
    // input_ptr is a *mut T that has been created from a Box<T>.
    // Converting it to a Box<MaybeUninit<T>> is safe, and will make sure that any drop-method
    // of T is not run. We must not drop T here, since we've moved it to the 'result_ptr'.
    let _t: Box<ManuallyDrop<T>> = unsafe { Box::from_raw(cur_ptr as *mut ManuallyDrop<T>) }; //Free the memory, but don't drop 'input'

    item_holder_ptr
}
fn get_full_ptr_raw<T: ?Sized, M: IMetadata>(
    dummy: *const ItemHolderDummy<T>,
) -> *const ItemHolder<T, M> {
    if is_sized::<T>() {
        assert_eq!(
            size_of::<*const ItemHolder<T, M>>(),
            size_of::<*const ItemHolderDummy<T>>()
        ); //<- Verify that *const T is not a fat ptr

        // SAFETY:
        // '*const ItemHolder<T, M>' is not _actually_ a fat pointer (it is just pointer sized),
        // so transmuting from dummy to it is correct.
        unsafe { core::mem::transmute_copy(&dummy) }
    } else {
        assert_eq!(size_of::<M>(), size_of::<usize>()); //<- Verify that *const ItemHolder<T> is a fat ptr

        let ptr_data = dummy as *mut _;
        debug_println!("Dummy data: {:?}", ptr_data);
        let metadata_ptr = dummy as *const UnsizedMetadata<T>;
        debug_println!(
            "Unsized, meta: {:?}, val: {:?}",
            metadata_ptr,
            // SAFETY:
            // metadata_ptr is a valid pointer
            unsafe { *metadata_ptr }
        );
        // SAFETY:
        // metadata_ptr is a valid pointer
        let metadata = unsafe { *metadata_ptr };
        arc_from_raw_parts_mut(ptr_data, metadata)
    }
}

pub(crate) struct SizedMetadata;
trait IMetadata {}
impl IMetadata for SizedMetadata {}
impl<T: ?Sized> IMetadata for UnsizedMetadata<T> {}

#[doc(hidden)]
#[repr(transparent)]
pub struct ItemHolderDummy<T: ?Sized> {
    // void pointers should point to u8
    _dummy: u8,
    _phantom_data: PhantomData<T>,
}

/*
Invariants:
 * ItemHolder cannot be deallocated if weak-count > 0
 * When strong-count goes from 0->1, weak-count must be incremented by 1
 * When strong-count goes from 1->0, weak-count must be decremented by 1
 * The rightmost ItemHolder cannot be dropped if there are other items (to the left).
   (But it may be possible to apply the janitor operation and drop them)
 * An ItemHolder can have its strong-count increased even if it is dropped,
   so just because strong-count is > 0 doesn't mean the item isn't dropped.
 * If strong-count > 0, weak-count is also > 0.
 * ItemHolder cannot be dropped if item.prev.advance_count > 0
 * Linked list defined by prev-pointers is the nominal order of the chain
 * ItemHolder can only be dropped when holding a lock on item,
   and a lock on item 'item.next', after having set 'item.prev.next' to 'item.next' (or further to the right).
   See do_advance_impl for details regarding 'advance_count'.
 * While dropping, the janitor process must be run. If a concurrent janitor task is detected,
   it must be marked as 'disturbed', and the disturbed task must re-run from the beginning
   after completing.
 * The weak-count has encoded in it 2 flags: HAVE_WEAK_NEXT and HAVE_WEAK_PREV. These decide
   if the item has an item to the right (next) and/or to the left (prev). The HAVE_WEAK_NEXT
   means there is, or soon will be, a neighbor to the right. HAVE_WEAK_PREV there isn't,
   or soon won't be, a neighbor to the left. When using compare_exchange to set the weak_count
   to 0, the code will atomically ensure no prev/next exist and that count is 0, in one operation.

*/

/// This structure represents the heap allocation in which ArcShift payloads reside.
/// Align 8 is needed, since we store flags in the lower 2 bits of the ItemHolder-pointers
/// In practice, the alignment of ItemHolder is 8 anyway, but we specify it here for clarity.
#[repr(align(8))]
#[repr(C)] // Just to get the 'magic' first and last in memory. Shouldn't hurt.
struct ItemHolder<T: ?Sized, M: IMetadata> {
    the_meta: M,
    #[cfg(feature = "validate")]
    magic1: std::sync::atomic::AtomicU64,
    /// Can be incremented to keep the 'next' value alive. If 'next' is set to a new value, and
    /// 'advance_count' is read as 0 after, then this node is definitely not going to advance to
    /// some node *before* 'next'.
    advance_count: atomic::AtomicUsize,

    /// Pointer to the prev value, or null.
    prev: atomic::AtomicPtr<ItemHolderDummy<T>>,
    /// Strong count. When 0, the item is dropped.
    /// Count must never be incremented from 0. This means increment must be done with
    /// compare-exchange, not fetch_add.
    strong_count: atomic::AtomicUsize,
    /// Weak count. When reaches 0, the 'janitor' task may deallocate the node, iff
    /// there are no previous nodes with 'advance_count' > 0.
    /// The next node holds a weak ref to the prev. But prev nodes don't
    /// hold weak counts on next.
    /// MSB of 'weak_count' is a 'nonext' flag, that tells that this item is the last in the
    /// chain. It is set to 1 *before* 'next' is assigned, so this is 0 when it is known
    /// that this won't be the last ever.
    weak_count: atomic::AtomicUsize,
    #[cfg(feature = "validate")]
    magic2: std::sync::atomic::AtomicU64,

    /// Pointer to the next value or null. Possibly decorated (i.e, least significant bit set)
    /// The decoration determines:
    ///  * If janitor process is currently active for this node and those left of it
    ///  * If payload has been deallocated
    ///  * The current lock-holder (janitor) has been disturbed (i.e, some other thread couldn't
    ///    do its job because it encountered the lock, and set the disturbed flag).
    next: atomic::AtomicPtr<ItemHolderDummy<T>>,
    pub(crate) payload: UnsafeCell<ManuallyDrop<T>>,
}

impl<T: ?Sized, M: IMetadata> PartialEq for ItemHolder<T, M> {
    #[allow(clippy::ptr_eq)] //We actually want to use addr_eq, but it's not available on older rust versions. Let's just use this for now.
    fn eq(&self, other: &ItemHolder<T, M>) -> bool {
        self as *const _ as *const u8 == other as *const _ as *const u8
    }
}

impl<T: ?Sized, M: IMetadata> ItemHolder<T, M> {
    fn set_next(&self, undecorated_new_next: *const ItemHolderDummy<T>) {
        #[cfg(feature = "validate")]
        assert_eq!(
            get_decoration(undecorated_new_next),
            ItemStateEnum::UndisturbedUndecorated
        );

        // Synchronizes with:
        // * 'has_payload' - irrelevant, has payload only cares about 'Dropped'-flag, not changed here
        // * 'lock_node_for_gc' - irrelevant, we already own the lock, and we're not changing the lock-bit
        // * 'do_upgrade_weak'
        //
        // Lock-free because we only loop if 'next' changes, which means
        // some other node has made progress.
        let mut prior_next = self.next.load(atomic::Ordering::Relaxed);
        loop {
            let new_next = decorate(undecorated_new_next, get_decoration(prior_next));
            match self.next.compare_exchange(
                prior_next,
                new_next as *mut _,
                atomic::Ordering::SeqCst, //atomic set_next
                atomic::Ordering::SeqCst,
            ) {
                Ok(_) => {
                    debug_println!(
                        "set {:x?}.next = {:x?}",
                        to_dummy(self as *const _),
                        new_next
                    );
                    return;
                }
                Err(err) => {
                    prior_next = err;
                }
            }
        }
    }

    /// Includes self
    #[allow(unused)]
    #[cfg(test)]
    #[cfg(feature = "std")]
    fn debug_all_to_left<'a>(&self) -> std::vec::Vec<&'a ItemHolder<T, M>> {
        let mut ret = alloc::vec::Vec::new();
        let mut item_ptr = self as *const ItemHolder<T, M>;
        // Lock-free because this loop is run in debug harness, and doesn't do any synchronization
        loop {
            // SAFETY:
            // item_ptr has just been created from the reference 'self', or was a non-null
            // 'prev' pointer of some valid reference, recursively. This method is only called
            // from the debug_validate machinery, that is 'unsafe' and requires the user to
            // make sure no concurrent access is occurring. At rest, 'prev' pointers are always
            // valid.
            ret.push(unsafe { &*item_ptr });
            // SAFETY:
            // See above. item_ptr is a valid pointer.
            let prev = from_dummy::<T, M>(unsafe { (*item_ptr).prev.load(Ordering::Relaxed) });
            if prev.is_null() {
                break;
            }
            item_ptr = prev;
        }

        ret
    }

    #[inline(always)]
    unsafe fn payload<'a>(&self) -> &'a T {
        // SAFETY:
        // Lifetime extension of shared references like this is permissible, as long as the object,
        // remains alive. Because of our refcounting, the object is kept alive as long as
        // the accessor (ArcShift-type) that calls this remains alive.
        unsafe { transmute::<&T, &'a T>(&*(self.payload.get())) }
    }

    // SAFETY:
    // Node must be uniquely owned, and payload must never be accessed again
    #[cfg(not(any(feature = "std", feature = "nostd_unchecked_panics")))]
    unsafe fn take_boxed_payload(&self) -> Box<T> {
        let payload = &self.payload;
        debug_println!("boxing payload");

        // SAFETY: item_ptr is now uniquely owned. We can drop it.
        let payload_val: &mut T = unsafe { &mut **(&mut *payload.get()) };
        let size = size_of_val(payload_val);
        let align = align_of_val(payload_val);
        let layout = Layout::from_size_align(size, align).unwrap();
        let thin_dest_ptr = if size == 0 {
            1 as *mut u8
        } else {
            alloc::alloc::alloc(layout)
        };

        let fat_dest_ptr = if is_sized::<T>() {
            core::ptr::copy_nonoverlapping(payload_val as *mut _ as *mut u8, thin_dest_ptr, size);
            core::mem::transmute_copy(&thin_dest_ptr)
        } else {
            core::ptr::copy_nonoverlapping(payload_val as *mut _ as *mut u8, thin_dest_ptr, size);
            let meta = UnsizedMetadata::new(payload_val);
            ptr_from_raw_parts_mut(thin_dest_ptr, meta)
        };

        Box::from_raw(fat_dest_ptr)
    }

    /// Returns true if the node could be atomically locked
    fn lock_node_for_gc(&self) -> bool {
        // Lock free, see comment for each 'continue' statement, which are the only statements
        // that lead to looping.
        loop {
            // This can't be 'Relaxed', because if we read it as already locked and disturbed,
            // we must make sure that this is still the case at this point in the total order.
            // When dropping, after exiting here, we won't clean up the entire chain. Our current
            // thread is not going to stop this from occurring (we've advanced to the rightmost), but
            // if it doesn't do it itself, we must make sure some other thread does it.
            //
            // If we read a stale value here, it may have been written by some other node _before_
            // we advanced, in which case that node may have already retried the GC, and failed
            // (since we hadn't advanced). This would then lead to a memory leak.
            let cur_next = self.next.load(atomic::Ordering::SeqCst);

            let decoration = get_decoration(cur_next);
            debug_println!(
                "loaded  {:x?}.next, got {:?} (decoration: {:?})",
                self as *const ItemHolder<T, M>,
                cur_next,
                decoration
            );
            if decoration.is_unlocked() {
                let decorated = decorate(undecorate(cur_next), decoration.with_gc());
                let success = self
                    .next
                    .compare_exchange(
                        cur_next,
                        decorated as *mut _,
                        Ordering::SeqCst, //atomic gc lock
                        atomic::Ordering::SeqCst,
                    )
                    .is_ok();
                debug_println!(
                    "Locking node {:x?}, result: {} (prior decoration: {:?}, new {:?})",
                    self as *const ItemHolder<T, M>,
                    success,
                    decoration,
                    get_decoration(decorated)
                );
                if !success {
                    // Lock free, because we only end up here if some other thread did one of:
                    // 1) added a new node, which is considered progress
                    // 2) has locked the node, which is not progress, but will only lead to us
                    //    setting the 'disturbed' flag, which is only one step, and thus lock free.
                    continue;
                }
                return true;
            } else {
                debug_println!(
                    "Locking node {:x?} failed, already decorated: {:?}",
                    self as *const ItemHolder<T, M>,
                    decoration
                );
                if !decoration.is_disturbed() {
                    let decorated = decorate(undecorate(cur_next), decoration.with_disturbed());
                    match self.next.compare_exchange(
                        cur_next,
                        decorated as *mut _,
                        Ordering::SeqCst, //atomic gc disturb
                        atomic::Ordering::SeqCst,
                    ) {
                        Ok(_) => {
                            debug_println!("Locking node {:x?} failed, set disturbed flag, new value: {:x?} (={:?})", self as * const ItemHolder<T, M>, decorated, get_decoration(decorated));
                        }
                        Err(prior) => {
                            if !get_decoration(prior).is_disturbed() {
                                // Lock free, because we can only end up here if some other thread did either:
                                // 1) Add a new node, which is considered progress
                                // 2) Removed the lock, which will lead to the other node advancing
                                //    to new things (which is progress), unless it was disturbed.
                                //    However, if it was disturbed, it wasn't due to us, since we
                                //    failed to set the disturbed flag.
                                continue;
                            }
                        }
                    }
                }

                // Already decorated
                return false;
            }
        }
    }

    /// Return true if node was disturbed
    #[must_use] //Not safe to leave a re-run request un-handled
    fn unlock_node(&self) -> bool {
        debug_println!("Unlocking node {:x?}", self as *const ItemHolder<T, M>);
        // Lock free, because we only loop if some other node modified 'next',
        // The only possible modifications are:
        // 1) Adding a new node, which is considered progress.
        // 2) Setting the disturbed flag, which means the other node has offloaded
        //    work on us, and is now proceeding to is next task, which means there is
        //    system progress, so algorithm remains lock free.
        loop {
            let cur_next = self.next.load(atomic::Ordering::Relaxed);
            let decoration = get_decoration(cur_next);
            let undecorated = decorate(undecorate(cur_next), decoration.without_gc_and_disturbed());

            #[cfg(feature = "validate")]
            assert!(
                decoration.is_locked(),
                "node {:x?} was not actually locked, decoration: {:?}",
                self as *const _,
                decoration
            );

            debug_println!("Unlocked dec {:x?}: {:x?}", self as *const Self, decoration);
            if self
                .next
                .compare_exchange(
                    cur_next,
                    undecorated as *mut _,
                    Ordering::SeqCst, //atomic gc unlock
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return decoration.is_disturbed();
            }
        }
    }
}

#[cfg(all(feature = "std", any(loom, feature = "shuttle"), feature = "validate"))]
static MAGIC: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);

impl<T: ?Sized, M: IMetadata> ItemHolder<T, M> {
    #[cfg_attr(test, mutants::skip)]
    #[cfg(feature = "validate")]
    fn verify(ptr2: *const ItemHolder<T, M>) {
        {
            let ptr = ptr2 as *const ItemHolder<(), M>;

            // SAFETY:
            // This function is never called with a null-ptr. Also, this is
            // just used for testing.
            let atomic_magic1 = unsafe { &*std::ptr::addr_of!((*ptr).magic1) };
            // SAFETY:
            // This function is never called with a null-ptr. Also, this is
            // just used for testing.
            let atomic_magic2 = unsafe { &*std::ptr::addr_of!((*ptr).magic2) };

            let magic1 = atomic_magic1.load(Ordering::Relaxed);
            let magic2 = atomic_magic2.load(Ordering::Relaxed);
            if magic1 >> 16 != 0xbeefbeefbeef {
                debug_println!(
                    "Internal error - bad magic1 in {:?}: {} ({:x})",
                    ptr,
                    magic1,
                    magic1
                );
                debug_println!("Backtrace: {}", std::backtrace::Backtrace::capture());
                std::process::abort();
            }
            if magic2 >> 16 != 0x123412341234 {
                debug_println!(
                    "Internal error - bad magic2 in {:?}: {} ({:x})",
                    ptr,
                    magic2,
                    magic2
                );
                debug_println!("Backtrace: {}", std::backtrace::Backtrace::capture());
                std::process::abort();
            }
            #[cfg(not(any(loom, feature = "shuttle")))]
            {
                let m1 = magic1 & 0xffff;
                let m2 = magic2 & 0xffff;
                if m1 != 0x8111 || m2 != 0x8111 {
                    debug_println!("Internal error - bad magic in {:?} {:x} {:x}", ptr, m1, m2);
                }
            }

            #[cfg(any(loom, feature = "shuttle"))]
            {
                let diff = (magic1 & 0xffff) as isize - (magic2 & 0xffff) as isize;
                if diff != 0 {
                    debug_println!(
                        "Internal error - bad magics in {:?}: {} ({:x}) and {} ({:x})",
                        ptr,
                        magic1,
                        magic1,
                        magic2,
                        magic2
                    );
                    debug_println!("Backtrace: {}", std::backtrace::Backtrace::capture());
                    panic!();
                }
                let magic = MAGIC.fetch_add(1, Ordering::Relaxed);
                let magic = magic as i64 as u64;
                atomic_magic1.fetch_and(0xffff_ffff_ffff_0000, Ordering::Relaxed);
                atomic_magic2.fetch_and(0xffff_ffff_ffff_0000, Ordering::Relaxed);
                atomic_magic1.fetch_or(magic, Ordering::Relaxed);
                atomic_magic2.fetch_or(magic, Ordering::Relaxed);
            }
        }
    }
}

/// Check the magic values of the supplied pointer, validating it in a best-effort fashion
#[inline]
#[allow(unused)]
#[cfg_attr(test, mutants::skip)]
fn verify_item_impl<T: ?Sized, M: IMetadata>(_ptr: *const ItemHolderDummy<T>) {
    #[cfg(feature = "validate")]
    {
        assert_is_undecorated::<T, M>(_ptr);

        let ptr = _ptr;
        let x = undecorate(ptr);
        if x != ptr {
            panic!("Internal error in ArcShift: Pointer given to verify was decorated, it shouldn't have been! {:?}", ptr);
        }
        if x.is_null() {
            return;
        }
        ItemHolder::<T, M>::verify(from_dummy::<T, M>(x))
    }
}
/// Check the magic values of the supplied pointer, validating it in a best-effort fashion
#[inline]
#[cfg_attr(test, mutants::skip)]
fn verify_item<T: ?Sized>(_ptr: *const ItemHolderDummy<T>) {
    #[cfg(feature = "validate")]
    {
        if is_sized::<T>() {
            verify_item_impl::<T, SizedMetadata>(_ptr)
        } else {
            verify_item_impl::<T, UnsizedMetadata<T>>(_ptr)
        }
    }
}

const ITEM_STATE_LOCKED_FLAG: u8 = 1;
const ITEM_STATE_DROPPED_FLAG: u8 = 2;
const ITEM_STATE_DISTURBED_FLAG: u8 = 4;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(unused)]
enum ItemStateEnum {
    /// Pointer is not decorated
    UndisturbedUndecorated = 0,
    UndisturbedGcIsActive = 1,
    UndisturbedPayloadDropped = 2,
    UndisturbedPayloadDroppedAndGcActive = 3,
    DisturbedUndecorated = 4,
    DisturbedGcIsActive = 5,
    DisturbedPayloadDropped = 6,
    DisturbedPayloadDroppedAndGcActive = 7,
}

impl ItemStateEnum {
    #[allow(unused)]
    fn is_locked(self) -> bool {
        (self as u8 & ITEM_STATE_LOCKED_FLAG) != 0
    }
    fn is_disturbed(self) -> bool {
        (self as u8 & ITEM_STATE_DISTURBED_FLAG) != 0
    }
    fn is_unlocked(self) -> bool {
        (self as u8 & ITEM_STATE_LOCKED_FLAG) == 0
    }
    fn dropped(self) -> ItemStateEnum {
        // SAFETY:
        // All 3-bit values are legal ItemStateEnum values, so oring on 4 is ok
        unsafe { core::mem::transmute::<u8, ItemStateEnum>(self as u8 | ITEM_STATE_DROPPED_FLAG) }
    }
    fn with_gc(self) -> ItemStateEnum {
        // SAFETY:
        // All 3-bit values are legal ItemStateEnum values, so oring on 1 is ok
        unsafe { core::mem::transmute::<u8, ItemStateEnum>(self as u8 | ITEM_STATE_LOCKED_FLAG) }
    }
    fn with_disturbed(self) -> ItemStateEnum {
        // SAFETY:
        // All 3-bit values are legal ItemStateEnum values, so oring on 1 is ok
        unsafe { core::mem::transmute::<u8, ItemStateEnum>(self as u8 | ITEM_STATE_DISTURBED_FLAG) }
    }
    fn without_gc_and_disturbed(self) -> ItemStateEnum {
        // SAFETY:
        // All 3-bit values are legal ItemStateEnum values, so anding out 1 is ok
        unsafe {
            core::mem::transmute::<u8, ItemStateEnum>(
                self as u8 & (!ITEM_STATE_LOCKED_FLAG) & (!ITEM_STATE_DISTURBED_FLAG),
            )
        }
    }
    fn is_dropped(self) -> bool {
        (self as u8 & ITEM_STATE_DROPPED_FLAG) != 0
    }
}

fn decorate<T: ?Sized>(
    ptr: *const ItemHolderDummy<T>,
    e: ItemStateEnum,
) -> *const ItemHolderDummy<T> {
    let curdecoration = (ptr as usize) & 7;
    ((ptr as *const u8).wrapping_offset((e as isize) - (curdecoration as isize)))
        as *const ItemHolderDummy<T>
}

/// Get the state encoded in the decoration, if any.
/// Returns None if the pointer is undecorated null.
/// The pointer must be valid.
fn get_decoration<T: ?Sized>(ptr: *const ItemHolderDummy<T>) -> ItemStateEnum {
    if ptr.is_null() {
        return ItemStateEnum::UndisturbedUndecorated;
    }
    let raw = ((ptr as usize) & 7) as u8;
    // SAFETY:
    // All values `0..=7` are valid ItemStateEnum.
    // And the bitmask produces a value `0..=7`
    unsafe { core::mem::transmute::<u8, ItemStateEnum>(raw) }
}

/// Panic if the pointer is decorated
#[allow(unused)]
#[cfg_attr(test, mutants::skip)]
#[inline]
fn assert_is_undecorated<T: ?Sized, M: IMetadata>(_ptr: *const ItemHolderDummy<T>) {
    #[cfg(feature = "validate")]
    {
        let raw = _ptr as usize & 7;
        if raw != 0 {
            panic!("Internal error in ArcShift - unexpected decorated pointer");
        }
    }
}

/// Return an undecorated version of the given pointer.
/// Supplying an already undecorated pointer is not an error, and returns
/// the value unmodified.
#[inline(always)]
fn undecorate<T: ?Sized>(cand: *const ItemHolderDummy<T>) -> *const ItemHolderDummy<T> {
    let raw = cand as usize & 7;
    (cand as *const u8).wrapping_offset(-(raw as isize)) as *const ItemHolderDummy<T>
}

impl<T: ?Sized> Clone for ArcShift<T> {
    fn clone(&self) -> Self {
        let t = with_holder!(self.item, T, item, {
            let mut drop_jobs = DropHandler::default();
            let result = do_clone_strong(item, &mut drop_jobs);
            drop_jobs.resume_any_panics();
            result
        });
        ArcShift {
            // SAFETY:
            // The pointer returned by 'do_clone_strong' is always valid and non-null.
            item: unsafe { NonNull::new_unchecked(t as *mut _) },
            pd: PhantomData,
        }
    }
}
impl<T: ?Sized> Clone for ArcShiftWeak<T> {
    fn clone(&self) -> Self {
        let t = with_holder!(self.item, T, item, do_clone_weak(item));
        ArcShiftWeak {
            // SAFETY:
            // The pointer returned by 'do_clone_weak' is always valid and non-null.
            item: unsafe { NonNull::new_unchecked(t as *mut _) },
        }
    }
}

fn do_clone_strong<T: ?Sized, M: IMetadata>(
    item_ptr: *const ItemHolder<T, M>,
    drop_job_queue: &mut impl IDropHandler<T, M>,
) -> *const ItemHolderDummy<T> {
    // SAFETY:
    // do_clone_strong must be called with a valid item_ptr
    let item = unsafe { &*item_ptr };
    debug_println!(
        "Strong clone, about to access strong count of {:x?}",
        item_ptr
    );
    let strong_count = item.strong_count.fetch_add(1, Ordering::Relaxed);

    if strong_count > MAX_REF_COUNT {
        item.strong_count.fetch_sub(1, Ordering::Relaxed);
        panic!("strong ref count max limit exceeded")
    }

    debug_println!(
        "strong-clone, new strong count: {:x?} is {}",
        item_ptr,
        strong_count + 1
    );
    #[cfg(feature = "validate")]
    assert!(strong_count > 0);

    do_advance_strong::<T, M>(to_dummy::<T, M>(item_ptr), drop_job_queue)
}

fn do_clone_weak<T: ?Sized, M: IMetadata>(
    item_ptr: *const ItemHolder<T, M>,
) -> *const ItemHolderDummy<T> {
    // SAFETY:
    // do_clone_weak must be called with a valid item_ptr
    let item = unsafe { &*item_ptr };
    let weak_count = item.weak_count.fetch_add(1, Ordering::Relaxed);
    if get_weak_count(weak_count) > MAX_REF_COUNT {
        item.weak_count.fetch_sub(1, Ordering::Relaxed);
        panic!("weak ref count max limit exceeded")
    }
    debug_println!(
        "==> weak-clone, fetch_add, new weak count: {:x?} is {}",
        item_ptr,
        format_weak(weak_count + 1)
    );
    #[cfg(feature = "validate")]
    assert!(weak_count > 0);
    to_dummy(item_ptr)
}

fn do_upgrade_weak<T: ?Sized, M: IMetadata>(
    item_ptr: *const ItemHolder<T, M>,
    jobq: &mut DropHandler<T>,
) -> Option<*const ItemHolderDummy<T>> {
    debug_println!("executing do_upgrade_weak");
    // SAFETY:
    // do_upgrade_weak must be called with a valid item_ptr
    let start_item = unsafe { &*(item_ptr) };
    {
        // This is needed, since ostensibly, this method works on a weak clone of 'item_ptr'.
        // It is this weak clone that is converted to a strong item.
        // A Relaxed add is sufficient here, since we don't need to synchronize with anything.
        // We already have a weak link. The extra weak link added here only becomes relevant
        // the next time we _decrease_ weak_count, and since that will be an op on the same
        // atomic, it will definitely see the value written here, which is all we require.
        let weak_count = start_item.weak_count.fetch_add(1, Ordering::Relaxed);
        if get_weak_count(weak_count) > MAX_REF_COUNT {
            start_item.weak_count.fetch_sub(1, Ordering::Relaxed);
            panic!("weak ref count max limit exceeded")
        }
        debug_println!(
            "==> do_upgrade_weak {:x?} incr weak count to {}",
            item_ptr,
            format_weak(weak_count + 1)
        );
    }
    let mut item_ptr = to_dummy(item_ptr);
    // Lock free: See comment on each 'continue'
    loop {
        item_ptr = do_advance_weak::<T, M>(item_ptr);
        // We still have one weak count on 'item_ptr' at this point. do_advance_weak
        // decrements the weak on the original value, and gives you an advanced value with
        // one count on.

        // SAFETY:
        // do_advance_weak always returns a valid non-null pointer
        let item = unsafe { &*from_dummy::<T, M>(item_ptr) };
        let prior_strong_count = item.strong_count.load(Ordering::SeqCst); //atomic upgrade read strong
        let item_next = item.next.load(Ordering::SeqCst); //atomic upgrade read next

        if !undecorate(item_next).is_null() {
            // Race, there's a new node we need to advance to.
            continue;
        }

        if !get_decoration(item_next).is_dropped() {
            if prior_strong_count == 0 {
                let _prior_weak_count = item.weak_count.fetch_add(1, Ordering::SeqCst); //atomic upgrade inc weak
                debug_println!(
                    "==> pre-upgrade strong=0 {:x?}, increase weak to {}",
                    item_ptr,
                    get_weak_count(_prior_weak_count) + 1
                );
            }

            if item
                .strong_count
                .compare_exchange(
                    prior_strong_count,
                    prior_strong_count + 1,
                    Ordering::SeqCst, //atomic upgrade inc strong
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                // SAFETY:
                // item_ptr has been advanced to, and we thus hold a weak ref.
                // it is a valid pointer.
                let item = unsafe { &*from_dummy::<T, M>(item_ptr) };
                let item_next = item.next.load(Ordering::SeqCst); //atomic upgrade check next drop
                if !undecorate(item_next).is_null() {
                    let new_prior_strong = item.strong_count.fetch_sub(1, Ordering::SeqCst); //atomic upgrade fail dropped
                    if new_prior_strong == 1 {
                        let _prior_weak_count = item.weak_count.fetch_sub(1, Ordering::SeqCst); //atomic upgrade fail dec last strong
                        #[cfg(feature = "validate")]
                        assert!(get_weak_count(_prior_weak_count) > 1);
                        debug_println!(
                            "==> {:x?} reduced weak to {}",
                            item_ptr,
                            get_weak_count(_prior_weak_count - 1)
                        );
                    }
                    // Lock free. We only get here if some other node has added a new node,
                    // which is considered progress.
                    continue;
                }

                let _reduce_weak = item.weak_count.fetch_sub(1, Ordering::SeqCst); //atomic upgrade sub weak
                debug_println!(
                    "==> upgrade success, new strong_count: {}, new weak: {}",
                    prior_strong_count + 1,
                    format_weak(_reduce_weak - 1)
                );
                #[cfg(feature = "validate")]
                assert!(
                    get_weak_count(_reduce_weak) > 1,
                    "assertion failed: in do_upgrade_weak(1), reduced weak {:x?} to 0",
                    item_ptr
                );

                return Some(item_ptr);
            } else {
                if prior_strong_count == 0 {
                    let _prior_weak_count = item.weak_count.fetch_sub(1, Ordering::SeqCst); //atomic upgrade race dec weak
                    debug_println!(
                        "==> pre-upgrade strong=0 race2 {:x?}, decrease weak to {}",
                        item_ptr,
                        get_weak_count(_prior_weak_count) - 1
                    );
                    #[cfg(feature = "validate")]
                    assert!(get_weak_count(_prior_weak_count) > 1);
                }
                debug_println!("Race on strong_count _increase_ - loop.");
                // Lcok free. We only get here if some other node has succeeded in increasing or
                // decreasing strong count, which only happens if there is progress, unless
                // the node undid a strong_count modification because of a detected race.
                // The possible strong_count undo operations are:
                // 1) If ArcShift::clone exceeds the maximum strong count limit. This is an
                //    error case, but we'll also consider it progress.
                // 2) If ArcShift::upgrade detects a race on 'next' (new node added), which
                //    is considered system-wide progress.
                // 3) In ArcShift::update, if it races with adding a new node, which is
                //    considered progress.
                continue; //Race on strong count, try again
            }
        } else {
            do_drop_weak::<T, M>(item_ptr, jobq);
            return None;
        }
    }
}

/// Returns an up-to-date pointer.
/// The closure is called with references to the old and new items, and allows
/// the caller to, for instance, decrease/inrcease strong counts. If no advance could be done,
/// this is always because we're already at the most up-to-date value. The closure is not called
/// in this case.
/// If the update closure fails, this can only be because there's a new 'next', and the
/// current item has been dropped.
///
/// This function requires the caller to have some sort of reference to 'item_ptr', so that
/// it can't go away during execution. This method does not touch that reference count.
///
/// The 'update' closure is called with a weak-ref taken on 'b', and 'a' in its original state.
/// It must either keep the weak ref, or possibly convert it to a strong ref.
/// It must also release whatever refcount it had on the original 'item_ptr'.
///
/// Guaranteed to return a valid, non-null pointer
fn do_advance_impl<T: ?Sized, M: IMetadata>(
    mut item_ptr: *const ItemHolderDummy<T>,
    mut update: impl FnMut(*const ItemHolder<T, M>, *const ItemHolder<T, M>) -> bool,
) -> *const ItemHolderDummy<T> {
    let start_ptr = item_ptr;
    verify_item(item_ptr);

    debug_println!("advancing from {:x?}", item_ptr);
    // Lock free, because we traverse the chain and eventually reach the end,
    // except that we may fail 'update', in which case we loop.
    // However, update fails only if there has been system-wide progress (see comment in
    // do_advance_strong).
    loop {
        debug_println!("In advance-loop, item_ptr: {:x?}", item_ptr);
        // SAFETY:
        // item_ptr is a pointer we have advanced to. Forward advance always visits only
        // valid pointers. This is a core algorithm of ArcShift. By the careful atomic operations
        // below, and by knowing how the janitor-task works, we make sure that either 'advance_count'
        // or 'weak_count' keep all visited references alive, such that they cannot be garbage
        // collected.
        let item: &ItemHolder<T, M> = unsafe { &*from_dummy(item_ptr as *mut _) };

        let next_ptr = undecorate(item.next.load(Ordering::SeqCst)); //atomic advance load next
        debug_println!("advancing from {:x?}, next_ptr = {:x?}", item_ptr, next_ptr);
        if next_ptr.is_null() {
            #[allow(clippy::collapsible_if)]
            if item_ptr != start_ptr {
                if !update(from_dummy(start_ptr), from_dummy(item_ptr)) {
                    debug_println!("upgrade failed, probably no payload");
                    continue;
                }
            }
            return item_ptr;
        }

        let _advanced = item.advance_count.fetch_add(1, Ordering::SeqCst); //atomic advance inc advance_count
        debug_println!(
            "advance: Increasing {:x?}.advance_count to {}",
            item_ptr,
            _advanced + 1
        );

        // This fence shouldn't be necessary, but without it loom fails, because
        // it doesn't actually model 'SeqCst' correctly. In principle, we could
        // have this fence only when running under loom, but then we'd not be testing
        // exactly the same thing as we run in prod, which seems unfortunate.
        //
        // Specifically, without this fence, loom might reorder this load before
        // the 'fetch_add' above, opening up the possibility that 'item.next' may be
        // dropped before we can do 'advance_count', and we'd still get that 'next'
        // in the load below.
        atomic::fence(Ordering::SeqCst);

        // We must reload next, because it's only the next that is _now_ set that is actually
        // protected by 'advance_count' above!
        let next_ptr = undecorate(item.next.load(Ordering::SeqCst)); //atomic advance reload next

        #[cfg(feature = "validate")]
        assert_ne!(next_ptr, item_ptr);
        // SAFETY:
        // next_ptr is guarded by our advance_count ref that we grabbed above.
        let next: &ItemHolder<T, M> = unsafe { &*from_dummy(next_ptr) };
        debug_println!("advance: Increasing next(={:x?}).weak_count", next_ptr);
        let _res = next.weak_count.fetch_add(1, Ordering::SeqCst); //atomic
        debug_println!(
            "==> do_advance_impl: increment weak of {:?}, was {}, now {}",
            next_ptr,
            format_weak(_res),
            format_weak(_res + 1),
        );

        let _advanced = item.advance_count.fetch_sub(1, Ordering::SeqCst); //atomic advance dec advance_count
        debug_println!(
            "advance: Decreasing {:x?}.advance_count to {}",
            item_ptr,
            _advanced - 1
        );

        if item_ptr != start_ptr {
            debug_println!("do_advance_impl: decrease weak from {:x?}", item_ptr);
            let _res = item.weak_count.fetch_sub(1, Ordering::SeqCst); //atomic advance dec weak_count
            debug_println!(
                "==> do_advance_impl: decrease weak from {:x?} - decreased to {}",
                item_ptr,
                format_weak(_res - 1)
            );

            // This should never reach 0 here. We _know_ that 'item' has a 'next'. Thus, this 'next'
            // must hold a reference count on this.
            #[cfg(feature = "validate")]
            assert!(
                get_weak_count(_res) > 1,
                "non rightmost node {:x?} can't be left with 0 weak count, but weak became {}",
                item_ptr,
                get_weak_count(_res) - 1
            );
        }
        item_ptr = next_ptr;
    }
}

// Guaranteed to return a valid, non-null pointer
fn do_advance_weak<T: ?Sized, M: IMetadata>(
    item_ptr: *const ItemHolderDummy<T>,
) -> *const ItemHolderDummy<T> {
    do_advance_impl(
        item_ptr,
        |a: *const ItemHolder<T, M>, _b: *const ItemHolder<T, M>| {
            // SAFETY:
            // a is a valid pointer. do_advance_impl only supplies usable pointers to the callback.
            let _a_weak = unsafe { (*a).weak_count.fetch_sub(1, Ordering::SeqCst) }; //atomic advance weak dec weak_count a

            // We have a weak ref count on 'b' given to use by `do_advance_impl`,
            // which we're fine with this and don't need to adjust b.weak_count

            debug_println!(
                "==> weak advance {:x?}, decremented weak counts to a:{:x?}={}",
                a,
                a,
                format_weak(_a_weak.wrapping_sub(1)),
            );
            #[cfg(feature = "validate")]
            assert!(get_weak_count(_a_weak) > 1);
            true
        },
    )
}

// Always returns valid, non-null, pointers
#[inline(never)]
fn do_advance_strong<T: ?Sized, M: IMetadata>(
    item_ptr: *const ItemHolderDummy<T>,
    drop_job_queue: &mut impl IDropHandler<T, M>,
) -> *const ItemHolderDummy<T> {
    do_advance_impl::<_, M>(item_ptr, move |a, b| {
        // SAFETY:
        // b is a valid pointer. do_advance_impl supplies the closure with only valid pointers.
        let mut b_strong = unsafe { (*b).strong_count.load(Ordering::SeqCst) }; //atomic advance strong load b strong_count

        // Lock free, since we only loop when compare_exchange fails on 'strong_count', something
        // which only occurs when 'strong_count' changes, which only occurs when there is
        // system wide progress.
        //
        // Note, do_advance_strong_impl will loop if this returns false. See each
        // such return for an argument why this only happens if there's been system side
        // progress.
        loop {
            debug_println!(
                "do_advance_strong {:x?} -> {:x?} (b-count = {})",
                a,
                b,
                b_strong
            );

            if b_strong == 0 {
                // a strong ref must _always_ imply a weak count, and that weak count must
                // always be decrementable by whoever happens to decrement the count to 0.
                // That might not be us (because we might race with another invocation of this code),
                // that might also increment the count.
                // SAFETY:
                // b is a valid pointer (see above)
                let _prior_weak = unsafe { (*b).weak_count.fetch_add(1, Ordering::SeqCst) }; //atomic advance strong inc b weak_count
                debug_println!(
                    "==> Prior to strong_count increment {:x?} from 0, added weak count. Weak now: {}",
                    b,
                    format_weak(_prior_weak + 1)
                );
            }

            // SAFETY:
            // b is a valid pointer. See above.
            match unsafe {
                (*b).strong_count.compare_exchange(
                    b_strong,
                    b_strong + 1,
                    Ordering::SeqCst, //atomic advance icn b strong_count
                    atomic::Ordering::SeqCst,
                )
            } {
                Ok(_) => {
                    debug_println!(
                        "b-strong increased {:x?}: {} -> {}",
                        b,
                        b_strong,
                        b_strong + 1
                    );
                    // SAFETY:
                    // b is a valid pointer. See above.
                    let next_ptr = unsafe { (*b).next.load(Ordering::SeqCst) }; //atomic advance reload next
                    if !undecorate(next_ptr).is_null() {
                        // Race - even though _did_ have payload prior to us grabbing the strong count, it now might not.
                        // This can only happen if there's some other node to the right that _does_ have a payload.

                        // SAFETY:
                        // b is a valid pointer. See above.
                        let prior_strong =
                            unsafe { (*b).strong_count.fetch_sub(1, Ordering::SeqCst) }; //atomic advance fail dec b strong_count
                        debug_println!("do_advance_strong:rare race, new _next_ appeared when increasing strong count: {:?} (decr strong back to {}) <|||||||||||||||||||||||||||||||||||||||||||||||||||||||>", b, prior_strong -1);
                        #[cfg(feature = "validate")]
                        assert!(
                            prior_strong > 0,
                            "strong count {:x?} must be >0, but was 0",
                            b
                        );

                        // If b_strong was not 0, it had a weak count. Our increment of it above would have made it so that
                        // no other thread could observe it going to 0. After our decrement below, it might go to zero.
                        // If it goes to zero in our decrement, and b_strong was not 0, it falls on us to free the weak_count.
                        // Note, in the span from the increment to this decrement, it could be argued that the implicitly owned weak count
                        // is "wrong". However, this is not observable, since the weak count is definitely not 0 (this closure does own a
                        // weak ref).
                        if prior_strong == 1 {
                            // SAFETY:
                            // b is a valid pointer. See above.
                            let _b_weak_count =
                                unsafe { (*b).weak_count.fetch_sub(1, Ordering::SeqCst) }; //atomic advance fail dec weak_count
                            debug_println!("==> do_advance_strong:rare2 {:x?} reduced strong count back to 0, reduced weak to: {}", b, get_weak_count(_b_weak_count-1));
                            #[cfg(feature = "validate")]
                            assert!(get_weak_count(_b_weak_count) > 1);
                        }
                        debug_println!("race, other thread added new node");
                        // Lock free, we can only get here if another thread has added a new node.
                        return false;
                    }
                    #[cfg(feature = "validate")]
                    assert!(!get_decoration(next_ptr).is_dropped());

                    // SAFETY:
                    // b is a valid pointer. See above.
                    let _b_weak_count = unsafe { (*b).weak_count.fetch_sub(1, Ordering::SeqCst) }; //atomic advance dec weak_count
                    debug_println!(
                        "==> strong advance {:x?}, reducing free weak b-count to {:?}",
                        b,
                        format_weak(_b_weak_count - 1)
                    );
                    #[cfg(feature = "validate")]
                    assert!(get_weak_count(_b_weak_count) > 1);

                    // SAFETY:
                    // a is a valid pointer. See above.
                    let a_strong = unsafe { (*a).strong_count.fetch_sub(1, Ordering::SeqCst) }; //atomic advance dec a strong_count
                    #[cfg(feature = "validate")]
                    assert_ne!(a_strong, 0);
                    debug_println!(
                        "a-strong {:x?} decreased {} -> {}",
                        a,
                        a_strong,
                        a_strong - 1,
                    );
                    if a_strong == 1 {
                        do_drop_payload_if_possible(a, false, drop_job_queue);
                        // Remove the implicit weak granted by the strong
                        // SAFETY:
                        // a is a valid pointer. See above.
                        let _a_weak = unsafe { (*a).weak_count.fetch_sub(1, Ordering::SeqCst) }; //atomic advance drop payload a
                        debug_println!("==> do_advance_strong:maybe dropping payload of a {:x?} (because strong-count is now 0). Adjusted weak to {}", a, format_weak(_a_weak.wrapping_sub(1)));
                        #[cfg(feature = "validate")]
                        assert!(get_weak_count(_a_weak) > 0);
                    }
                    return true;
                }
                Err(err_value) => {
                    if b_strong == 0 {
                        // SAFETY:
                        // b is a valid pointer. See above.
                        let _prior_weak = unsafe { (*b).weak_count.fetch_sub(1, Ordering::SeqCst) }; //atomic advance race
                        debug_println!("==> do_advance_strong: race on strong_count for {:x?} (restore-decr weak to {})", b, get_weak_count(_prior_weak - 1));
                        #[cfg(feature = "validate")]
                        assert!(get_weak_count(_prior_weak) > 1);
                    }
                    b_strong = err_value;
                }
            }
        }
    })
}

fn raw_deallocate_node<T: ?Sized, M: IMetadata>(
    item_ptr: *const ItemHolder<T, M>,
    jobq: &mut impl IDropHandler<T, M>,
) {
    verify_item(to_dummy(item_ptr));
    raw_do_unconditional_drop_payload_if_not_dropped(item_ptr as *mut ItemHolder<T, M>, jobq);

    // SAFETY:
    // item_ptr is going to be deallocated, and the caller must make sure
    // that it has exclusive access.
    #[cfg(feature = "validate")]
    assert_eq!(
        // SAFETY:
        // item_ptr is guaranteed valid by caller
        unsafe { (*item_ptr).strong_count.load(Ordering::SeqCst) },
        0,
        "{:x?} strong count must be 0 when deallocating",
        item_ptr
    );

    #[cfg(feature = "validate")]
    {
        // SAFETY:
        // item_ptr is guaranteed valid by caller
        let item_ref = unsafe { &*(item_ptr as *mut ItemHolder<T, M>) };
        item_ref.magic1.store(0xdeaddead, Ordering::Relaxed);
        item_ref.magic2.store(0xdeaddea2, Ordering::Relaxed);
    }

    do_dealloc(item_ptr);
}

#[derive(PartialEq)]
enum NodeStrongStatus {
    // We saw nodes with strong refs, that weren't the rightmost node
    StrongRefsFound,
    // We visited all nodes all the way to the leftmost one, and there were no strong refs
    // to the left of the rightmost one.
    NoStrongRefsExist,
    // We had to stop our leftward traverse, because nodes were locked, so we couldn't
    // say for sure if strong refs existed. This means that some other node is running an
    // old gc, and upon completion of this gc it will realize it has to advance, so this
    // other node will have a chance to detect that no strong refs exist.
    Indeterminate,
}

fn do_janitor_task<T: ?Sized, M: IMetadata>(
    start_ptr: *const ItemHolder<T, M>,
    jobq: &mut impl IDropHandler<T, M>,
) -> (bool /*need re-run*/, NodeStrongStatus) {
    verify_item(to_dummy(start_ptr));

    let mut have_seen_left_strong_refs = false;
    fn get_strong_status(strong: bool, leftmost: bool) -> NodeStrongStatus {
        if strong {
            NodeStrongStatus::StrongRefsFound
        } else if leftmost {
            NodeStrongStatus::NoStrongRefsExist
        } else {
            NodeStrongStatus::Indeterminate
        }
    }

    debug_println!("Janitor task for {:x?}", start_ptr);
    // SAFETY:
    // start_ptr must be a valid pointer. This falls on the caller toensure.
    let start = unsafe { &*start_ptr };

    let start_ptr = to_dummy(start_ptr);

    if !start.lock_node_for_gc() {
        debug_println!("Janitor task for {:x?} - gc already active!", start_ptr);
        return (false, get_strong_status(have_seen_left_strong_refs, false));
    }

    debug_println!("Loading prev of {:x?}", start_ptr);
    let mut cur_ptr: *const _ = start.prev.load(Ordering::SeqCst); //atomic janitor load prev
    debug_println!("Loaded prev of {:x?}, got: {:x?}", start_ptr, cur_ptr);
    let rightmost_candidate_ptr = cur_ptr;
    if cur_ptr.is_null() {
        debug_println!("Janitor task for {:x?} - no earlier nodes exist", start_ptr);
        // There are no earlier nodes to clean up
        let need_rerun = start.unlock_node();
        return (
            need_rerun,
            get_strong_status(have_seen_left_strong_refs, true),
        );
    }

    let mut rightmost_deletable = null();

    // It may seems that we iterate through the chain in many different ways
    // in this method, and that it should be possible to abstract this.
    // However, it turns out to be slightly tricky, since the iteration is always
    // done in a very careful way to maintain correctness, and exchanging the order
    // of two innocent looking statements can cause it to fail.

    {
        let mut any_failed_locks = false;
        let cur_lock_start = cur_ptr;
        let mut cur_ptr = cur_ptr;
        debug_println!("Lock sweep start at {:x?}", cur_lock_start);
        let mut failed_lock = null();
        loop {
            // SAFETY: cur_ptr is a valid pointer It starts out equal to
            // 'start.prev', which is always valid or null, and we check it for null.
            // We then fetch 'cur.prev' pointers, which is valid, since we only do so if
            // we managed to take a lock. No other thread may free prev if it is locked.
            let cur: &ItemHolder<T, M> = unsafe { &*from_dummy(cur_ptr) };

            if !cur.lock_node_for_gc() {
                failed_lock = cur_ptr;
                any_failed_locks = true;
                debug_println!(
                    "Janitor task for {:x?} - found node already being gced: {:x?}",
                    start_ptr,
                    cur_ptr
                );
                break;
            }

            let prev_ptr: *const _ = cur.prev.load(Ordering::SeqCst); //atomic janitor load prev 2
            debug_println!("Janitor loop considering {:x?} -> {:x?}", cur_ptr, prev_ptr);
            if prev_ptr.is_null() {
                debug_println!(
                    "Janitor task for {:x?} - found leftmost node: {:x?}",
                    start_ptr,
                    cur_ptr
                );
                break;
            }
            cur_ptr = prev_ptr;
        }
        if any_failed_locks {
            let mut cur_ptr = cur_lock_start;
            debug_println!("Cleanup sweep start at {:x?}", cur_lock_start);
            let mut anyrerun = false;
            loop {
                debug_println!("Cleanup sweep at {:x?}", cur_ptr);
                if cur_ptr.is_null() {
                    anyrerun = true;
                    break;
                }
                if cur_ptr == failed_lock {
                    debug_println!("Cleanup sweep is at end {:x?}", cur_ptr);
                    break;
                }
                // SAFETY:
                // cur_ptr is a valid pointer. It's just the left traverse, across
                // a chain of locked nodes.
                let cur: &ItemHolder<T, M> = unsafe { &*from_dummy(cur_ptr) };

                let prev_ptr: *const _ = cur.prev.load(Ordering::SeqCst); //atomic janitor cleanup load prev
                anyrerun |= cur.unlock_node();
                debug_println!("Cleanup sweep going to {:x?}", prev_ptr);
                cur_ptr = prev_ptr;
            }
            anyrerun |= start.unlock_node();
            debug_println!("Janitor failed to grab locks. Rerun: {:?}", anyrerun);
            return (
                anyrerun,
                get_strong_status(have_seen_left_strong_refs, false),
            );
        }
    }

    // Lock free, since this just iterates through the chain of nodes,
    // which has a bounded length.
    loop {
        debug_println!("Accessing {:x?} in first janitor loop", cur_ptr);
        // SAFETY:
        // cur_ptr remains a valid pointer. Our lock on the nodes, as we traverse them backward,
        // ensures that no other janitor task can free them.
        let cur: &ItemHolder<T, M> = unsafe { &*from_dummy(cur_ptr) };
        debug_println!("Setting next of {:x?} to {:x?}", cur_ptr, start_ptr,);
        #[cfg(feature = "validate")]
        assert_ne!(cur_ptr, start_ptr);

        if cur.strong_count.load(Ordering::SeqCst) > 0 {
            //atomic janitor check strong_count
            have_seen_left_strong_refs = true;
        }

        let prev_ptr: *const _ = cur.prev.load(Ordering::SeqCst); //atomic janitor load prev 4
        debug_println!("Janitor loop considering {:x?} -> {:x?}", cur_ptr, prev_ptr);
        if prev_ptr.is_null() {
            debug_println!(
                "Janitor task for {:x?} - found leftmost node: {:x?}",
                start_ptr,
                cur_ptr
            );
            break;
        }

        // SAFETY:
        // We guarded 'cur_ptr', and it is thus alive. It holds a weak-ref on its 'prev', which
        // means that ref must now be keeping 'prev' alive. This means 'prev' is known to be a
        // valid pointer. We never decorate prev-pointers, so no undecorate is needed.
        let prev: &ItemHolder<T, M> = unsafe { &*from_dummy(prev_ptr) };
        #[cfg(feature = "validate")]
        assert_ne!(prev_ptr, start_ptr);

        prev.set_next(start_ptr);

        // We need a fence here, because otherwise this load could be reordered before the
        // `set_next`call above, while running under loom. This could lead to a situation
        // where a thread reads the previous next-pointer, and we still don't see the new
        // 'advance_count' here. With the fence, we know that either:
        // 1) The other thread has seen our new 'next' value supplied above
        // or
        // 2) We see their incremented 'advance_count'.
        // Note, we may thus block the deletion of next unnecessarily, if the other thread
        // read our new 'next' value just after we wrote it, and incremented 'advance_count'
        // here before we loaded it. This race condition is benign however, the other thread
        // will also try janitoring, and will delete the node if necessary.
        atomic::fence(Ordering::SeqCst);

        let adv_count = prev.advance_count.load(Ordering::SeqCst); //atomic janitor check advance_count
        debug_println!("advance_count of {:x?} is {}", prev_ptr, adv_count);
        if adv_count > 0 {
            debug_println!(
                "Janitor task for {:x?} - node {:x?} has advance in progress",
                start_ptr,
                prev_ptr
            );
            // All to the right of cur_ptr could be targets of the advance.
            // Because of races, we don't know that their target is actually 'start_ptr'
            // (though it could be).
            rightmost_deletable = cur_ptr;
        }
        cur_ptr = prev_ptr;
    }

    fn delete_nodes_that_can_be_deleted<T: ?Sized, M: IMetadata>(
        mut item_ptr: *const ItemHolderDummy<T>,
        jobq: &mut impl IDropHandler<T, M>,
    ) -> Option<*const ItemHolderDummy<T>> {
        let mut deleted_count = 0;

        // Lock free, since this loop at most iterates once per node in the chain.
        loop {
            if item_ptr.is_null() {
                debug_println!(
                    "Find non-deleted {:x?}, count = {}, item_ptr = {:x?}",
                    item_ptr,
                    deleted_count,
                    item_ptr,
                );
                return (deleted_count > 0).then_some(item_ptr);
            }
            // SAFETY:
            // caller must ensure item_ptr is valid, or null. And it is not null, we checked above.
            let item: &ItemHolder<T, M> = unsafe { &*from_dummy(item_ptr as *mut _) };
            // Item is *known* to have a 'next'!=null here, so
            // we know weak_count == 1 implies it is only referenced by its 'next'
            let item_weak_count = item.weak_count.load(Ordering::SeqCst); //atomic janitor load weak_count
            if get_weak_count(item_weak_count) > 1 {
                debug_println!(
                    "==> Find non-deleted {:x?}, count = {}, found node with weak count > 1 ({})",
                    item_ptr,
                    deleted_count,
                    format_weak(item_weak_count)
                );
                return (deleted_count > 0).then_some(item_ptr);
            }
            debug_println!(
                "==> Deallocating node in janitor task: {:x?}, dec: ?, weak count: {}",
                item_ptr,
                // SAFETY:
                // item_ptr is valid
                format_weak(item_weak_count),
            );

            #[cfg(feature = "validate")]
            {
                let prior_item_weak_count = item.weak_count.load(Ordering::SeqCst);
                #[cfg(feature = "validate")]
                assert_eq!(
                    get_weak_count(prior_item_weak_count),
                    1,
                    "{:x?} weak_count should still be 1, and we decrement it to 0",
                    item_ptr
                );
            }

            let prev_item_ptr = item.prev.load(Ordering::SeqCst); //atomic janitor deletion load prev
            raw_deallocate_node(from_dummy::<T, M>(item_ptr as *mut _), jobq);
            deleted_count += 1;
            item_ptr = prev_item_ptr;
        }
    }

    let mut cur_ptr = rightmost_candidate_ptr;
    let mut right_ptr = start_ptr;
    let mut must_see_before_deletes_can_be_made = rightmost_deletable;
    debug_println!("Starting final janitor loop");
    let mut need_rerun = false;
    // We must retain the locks one-past the node with the lock (lock works leftward)
    let mut unlock_carry: Option<*const ItemHolder<T, M>> = None;
    let mut do_unlock = |node: Option<*const ItemHolder<T, M>>| {
        if let Some(unlock) = unlock_carry.take() {
            // SAFETY:
            // Caller ensures pointer is valid.
            need_rerun |= unsafe { (*unlock).unlock_node() };
        }
        unlock_carry = node;
    };

    #[cfg(feature = "validate")]
    assert!(!cur_ptr.is_null());

    // Iterate through chain, deleting nodes when possible
    // Lock free, since this loop just iterates through each node in the chain.
    while !cur_ptr.is_null() {
        let new_predecessor = must_see_before_deletes_can_be_made
            .is_null()
            .then(|| delete_nodes_that_can_be_deleted::<T, M>(cur_ptr, jobq))
            .flatten();

        if cur_ptr == must_see_before_deletes_can_be_made {
            debug_println!("Found must_see node: {:x?}", cur_ptr);
            // cur_ptr can't be deleted itself, but nodes further to the left can
            must_see_before_deletes_can_be_made = null_mut();
        }

        if let Some(new_predecessor_ptr) = new_predecessor {
            // SAFETY:
            // right_ptr is always the start pointer, or the previous pointer we visited.
            // in any case, it is non-null and valid.
            let right: &ItemHolder<T, M> = unsafe { &*from_dummy(right_ptr) };
            debug_println!(
                "After janitor delete, setting {:x?}.prev = {:x?}",
                right_ptr,
                new_predecessor_ptr
            );
            if new_predecessor_ptr.is_null() {
                right
                    .weak_count
                    .fetch_and(!WEAK_HAVE_PREV, Ordering::SeqCst); //atomic janitor clear prev
            }
            right
                .prev
                .store(new_predecessor_ptr as *mut _, Ordering::SeqCst); //atomic janitor unlink

            if new_predecessor_ptr.is_null() {
                debug_println!("new_predecessor is null");
                break;
            }
            debug_println!("found new_predecessor: {:x?}", new_predecessor_ptr);
            right_ptr = new_predecessor_ptr;
            // SAFETY:
            // We only visit nodes we have managed to lock. new_predecessor is such a node
            // and it is not being deleted. It is safe to access.
            let new_predecessor = unsafe { &*from_dummy::<T, M>(new_predecessor_ptr) };
            cur_ptr = new_predecessor.prev.load(Ordering::SeqCst); //atomic janitor load prev 6
            #[cfg(feature = "validate")]
            assert_ne!(
                new_predecessor_ptr as *const _,
                null(),
                "assert failed, {:?} was endptr",
                new_predecessor_ptr
            );
            do_unlock(Some(new_predecessor));
            debug_println!("New candidate advanced to {:x?}", cur_ptr);
        } else {
            debug_println!("Advancing to left, no node to delete found {:x?}", cur_ptr);
            // SAFETY:
            // All the pointers we traverse (leftward) through the chain are valid,
            // since we lock them succesively as we traverse.
            let cur: &ItemHolder<T, M> = unsafe { &*from_dummy(cur_ptr) };
            right_ptr = cur_ptr;
            cur_ptr = cur.prev.load(Ordering::SeqCst); //atomic janitor load prev 7
            debug_println!("Advancing to left, advanced to {:x?}", cur_ptr);
            do_unlock(Some(cur));
        }
    }
    do_unlock(None);
    debug_println!("Unlock start-node {:x?}", start_ptr);
    need_rerun |= start.unlock_node();
    debug_println!("start-node unlocked: {:x?}", start_ptr);
    (
        need_rerun,
        get_strong_status(have_seen_left_strong_refs, true),
    )
}

/// drop_payload should be set to drop the payload of 'original_item_ptr' iff it is
/// either not rightmost, or all refs to all nodes are just weak refs (i.e, no strong refs exist)
fn do_drop_weak<T: ?Sized, M: IMetadata>(
    original_item_ptr: *const ItemHolderDummy<T>,
    drop_job_queue: &mut impl IDropHandler<T, M>,
) {
    debug_println!("drop weak {:x?}", original_item_ptr);
    let mut item_ptr = original_item_ptr;
    // Lock free, since we only loop in these cases:
    // 1) If the janitor task was disturbed. If it was, some other thread has offloaded
    //    work on us, which means the other thread could carry on and do other stuff, which
    //    means there is system wide progress.
    // 2) If a new 'next' pointer has been set, i.e, a new node has been added. This is
    //    considered progress.
    // 3) Some other node changed the weak_count. In that case, there are two cases:
    //   3a) That other node was running 'do_drop_weak'. After changing the weak_count
    //       there are no more loop conditions, so it has definitely made progress.
    //   3b) That node was running some other work. It managed to change the weak count,
    //       and is not running 'do_drop_weak', so the system is making progress as long as
    //       that other thread is making progress, which it must be (see all other comments),
    //       and it cannot have been obstructed by this thread.
    loop {
        debug_println!("drop weak loop {:?}", item_ptr);
        item_ptr = do_advance_weak::<T, M>(item_ptr);
        let (need_rerun, strong_refs) =
            do_janitor_task(from_dummy::<T, M>(item_ptr), drop_job_queue);
        if need_rerun {
            debug_println!("Janitor {:?} was disturbed. Need rerun", item_ptr);
            continue;
        }

        // When we get here, we know we are advanced to the most recent node

        debug_println!("Accessing {:?}", item_ptr);
        // SAFETY:
        // item_ptr is a pointer that was returned by 'do_advance_weak'. The janitor task
        // never deallocates the pointer given to it (only prev pointers of that pointer).
        // do_advance_weak always returns valid non-null pointers.
        let item: &ItemHolder<T, M> = unsafe { &*from_dummy(item_ptr) };
        let prior_weak = item.weak_count.load(Ordering::SeqCst); //atomic drop_weak load weak_count

        #[cfg(feature = "validate")]
        assert!(get_weak_count(prior_weak) > 0);

        let next_ptr = item.next.load(Ordering::SeqCst); //atomic drop_weak load next
        let have_next = !undecorate(next_ptr).is_null();
        if have_next {
            // drop_weak raced with 'add'.
            debug_println!(
                "add race {:x?}, prior_weak: {}",
                item_ptr,
                format_weak(prior_weak)
            );
            continue;
        }
        // Note, 'next' may be set here, immediately after us loading 'next' above.
        // This is benign. In this case, 'prior_weak' will also have get_weak_next(prior_weak)==true
        // and get_weak_count(prior_weak)>1. We will not drop the entire chain, but whoever
        // just set 'next' definitely will.

        #[cfg(feature = "debug")]
        if get_weak_next(prior_weak) {
            debug_println!("raced in drop weak - {:x?} was previously advanced to rightmost, but now it has a 'next' again (next is ?)", item_ptr);
        }

        debug_println!("No add race, prior_weak: {}", format_weak(prior_weak));

        // We now have enough information to know if we can drop payload, if desired
        {
            // SAFETY:
            // item_ptr was returned by do_advance_weak, and is thus valid.
            let o_item = unsafe { &*from_dummy::<T, M>(item_ptr) };
            let o_strong = o_item.strong_count.load(Ordering::SeqCst); //atomic drop_weak check strong_count
            if o_strong == 0 {
                let can_drop_now_despite_next_check = if strong_refs
                    == NodeStrongStatus::NoStrongRefsExist
                {
                    debug_println!("final drop analysis {:x?}: Can drop this payload, because no strong refs exists anywhere", item_ptr);
                    true
                } else {
                    debug_println!("final drop analysis {:x?}: no exemption condition found, can't drop this payload", item_ptr);
                    false
                };
                if can_drop_now_despite_next_check {
                    do_drop_payload_if_possible(
                        o_item,
                        can_drop_now_despite_next_check,
                        drop_job_queue,
                    );
                }
            }
        }

        match item.weak_count.compare_exchange(
            prior_weak,
            prior_weak - 1,
            Ordering::SeqCst, //atomic drop_weak dec weak
            Ordering::SeqCst,
        ) {
            Ok(_) => {
                debug_println!(
                    "==> drop weak {:x?}, did reduce weak to {}",
                    item_ptr,
                    format_weak(prior_weak - 1)
                );
            }
            Err(_err_weak) => {
                debug_println!(
                    "drop weak {:x?}, raced on count decrement (was {}, expected {})",
                    item_ptr,
                    format_weak(_err_weak),
                    format_weak(prior_weak)
                );

                // Note:
                // This case occurs if two threads race to drop a weak ref.
                // Crucially, it is also in play to distinguish between these two scenarios:
                // 1: Two nodes have weak refs. One of them (A) is dropped, the other (B) remains.
                // 2: Two nodes have weak refs. One of them (A) updates the value, and is then
                //    dropped. The other (B) remains.
                //
                // In both cases, when B is dropped, it will see a total weak count of 2.
                // However, in case 2, B must re-run 'advance', and do another janitor cycle.
                // This *has* to be checked during the weak_count compare_exchange, since doing
                // it at any point prior to that opens up the risk of a race where node A above
                // manages to update and drop before the compare-exchange.

                continue;
            }
        }

        debug_println!(
            "Prior weak of {:x?} is {}",
            item_ptr,
            format_weak(prior_weak)
        );
        if get_weak_count(prior_weak) == 1 {
            #[cfg(feature = "validate")]
            if get_weak_next(prior_weak) {
                std::eprintln!("This case should not be possible, since it implies the 'next' object didn't increase the refcount");
                std::process::abort();
            }
            // We know there's no next here either, since we checked 'get_weak_next' before the cmpxch, so we _KNOW_ we're the only user now.
            if !get_weak_prev(prior_weak) {
                debug_println!("drop weak {:x?}, prior count = 1, raw deallocate", item_ptr);
                drop_job_queue.report_sole_user();
                raw_deallocate_node(from_dummy::<T, M>(item_ptr), drop_job_queue);
            } else {
                debug_println!(
                    "drop weak {:x?}, couldn't drop node, because there are nodes to the left",
                    item_ptr
                );
                // These nodes will presumably realize they're holding leftwise refs, and before dropping
                // they must advance. And thus the whole chain _will_ be collected.
            }
        }
        return;
    }
}

fn do_update<T: ?Sized, M: IMetadata>(
    initial_item_ptr: *const ItemHolder<T, M>,
    mut val_dummy_factory: impl FnMut(&ItemHolder<T, M>) -> Option<*const ItemHolderDummy<T>>,
    drop_job_queue: &mut impl IDropHandler<T, M>,
) -> *const ItemHolderDummy<T> {
    let mut item_ptr = to_dummy::<T, M>(initial_item_ptr);
    verify_item(item_ptr);

    // Lock free. This loop only loops if another thread has changed 'next', which
    // means there is system wide progress.
    loop {
        item_ptr = do_advance_strong::<T, M>(item_ptr, drop_job_queue);

        // SAFETY:
        // item_ptr was returned by do_advance_strong, and is thus valid.
        let item = unsafe { &*from_dummy::<T, M>(item_ptr) };
        debug_println!(
            "Updating {:x?} , loading next of {:x?}",
            initial_item_ptr,
            item_ptr
        );

        let cur_next = item.next.load(Ordering::SeqCst); //atomic update check_next
        if !undecorate(cur_next).is_null() {
            continue;
        }
        let Some(val_dummy) = val_dummy_factory(item) else {
            return item_ptr;
        };
        let new_next = decorate(val_dummy, get_decoration(cur_next));

        debug_println!("Updating {:x?} to {:x?}", item_ptr, val_dummy);
        let val = from_dummy::<T, M>(val_dummy) as *mut ItemHolder<T, M>;
        // SAFETY:
        // val is the new value supplied by the caller. It is always valid, and it is not
        // yet linked into the chain, so no other node could free it.
        unsafe { (*val).prev = atomic::AtomicPtr::new(item_ptr as *mut _) };

        // We must increment weak_count here, because this is the signal that
        // causes the drop_weak impl to realize that it is racing with an 'update',
        // (in contrast to perhaps racing with another drop, in which case it might
        // win the race and might be required to deallocate, to avoid memory leaks).
        let _weak_was = item.weak_count.fetch_add(1, Ordering::SeqCst);

        #[cfg(feature = "validate")]
        assert!(_weak_was > 0);
        debug_println!(
            "do_update: increment weak of {:?} (weak now: {})",
            item_ptr,
            format_weak(_weak_was + 1)
        );

        debug_println!("bef item.next.compare_exchange");
        match item.next.compare_exchange(
            cur_next,
            new_next as *mut _,
            Ordering::SeqCst, //atomic update exchange next
            Ordering::SeqCst,
        ) {
            Ok(_) => {
                item.weak_count.fetch_or(WEAK_HAVE_NEXT, Ordering::SeqCst); //atomic update mark have next
                debug_println!("aft1 item.next.compare_exchange");
            }
            Err(_) => {
                debug_println!("aft2 item.next.compare_exchange");
                debug_println!("race, update of {:x?} to {:x?} failed", item_ptr, new_next);
                let _res = item.weak_count.fetch_sub(1, Ordering::SeqCst); //atomic update fail dec weak_count
                debug_println!(
                    "==> race, decreasing {:x?} weak to {}",
                    item_ptr,
                    format_weak(_res - 1)
                );
                #[cfg(feature = "validate")]
                assert!(get_weak_count(_res) > 1);

                continue;
            }
        }

        let strong_count = item.strong_count.fetch_sub(1, Ordering::SeqCst); //atomic update dec strong_count
        debug_println!(
            "do_update: strong count {:x?} is now decremented to {}",
            item_ptr,
            strong_count - 1,
        );
        if strong_count == 1 {
            // It's safe to drop payload here, we've just now added a new item
            // that has its payload un-dropped, so there exists something to advance to.
            do_drop_payload_if_possible(from_dummy::<T, M>(item_ptr), false, drop_job_queue);
            let _weak_count = item.weak_count.fetch_sub(1, Ordering::SeqCst); //atomic update dec weak_count
            debug_println!(
                "==> do_update: decrement weak of {:?}, new weak: {}",
                item_ptr,
                format_weak(_weak_count.saturating_sub(1))
            );
            #[cfg(feature = "validate")]
            assert!(get_weak_count(_weak_count) > 1);
        }

        item_ptr = val_dummy;

        //If the strong_count of the previous went to 0, do a janitor-cycle
        if strong_count == 1 {
            // Lock free
            // Only loops if another thread has set the 'disturbed' flag, meaning it has offloaded
            // work on us, and it has made progress, which means there is system wide progress.
            loop {
                let (need_rerun, _strong_refs) =
                    do_janitor_task(from_dummy::<T, M>(item_ptr), drop_job_queue);
                if need_rerun {
                    item_ptr = do_advance_strong::<T, M>(item_ptr, drop_job_queue);
                    debug_println!("Janitor {:?} was disturbed. Need rerun", item_ptr);
                    continue;
                }
                break;
            }
        }
        return item_ptr;
    }
}

// Drops payload, unless either:
// * We're the rightmost node
// * The payload is already dropped
// the item_ptr must be valid.
fn do_drop_payload_if_possible<T: ?Sized, M: IMetadata>(
    item_ptr: *const ItemHolder<T, M>,
    override_rightmost_check: bool,
    drop_queue: &mut impl IDropHandler<T, M>,
) {
    // SAFETY:
    // caller must supply valid pointer.
    let item = unsafe { &*(item_ptr) };
    // Lock free. This only loops if another thread has changed next, which implies
    // progress.
    loop {
        let next_ptr = item.next.load(Ordering::Relaxed);
        let decoration = get_decoration(next_ptr);
        if decoration.is_dropped() || (undecorate(next_ptr).is_null() && !override_rightmost_check)
        {
            debug_println!(
                "Not dropping {:x?} payload, because it's rightmost",
                item_ptr
            );
            return;
        }
        debug_println!(
            "Drop if possible, {:x?}, cur dec: {:?}, dropped dec {:?}",
            item_ptr,
            decoration,
            decoration.dropped()
        );
        match item.next.compare_exchange(
            next_ptr,
            decorate(undecorate(next_ptr), decoration.dropped()) as *mut _,
            Ordering::SeqCst, //atomic mark dropped
            Ordering::SeqCst,
        ) {
            Ok(_) => {
                break;
            }
            Err(err_ptr) => {
                if !get_decoration(err_ptr).is_dropped() {
                    debug_println!("Raced, new attempt to drop {:x?} payload", item_ptr);
                    continue;
                }
                debug_println!("Raced, {:x?} now already dropped", item_ptr);
                return;
            }
        }
    }
    debug_println!("Dropping payload {:x?} (dec: ?)", item_ptr,);
    // SAFETY: item_ptr must be valid, guaranteed by caller.
    drop_queue.do_drop(item_ptr as *mut _);
}

// Caller must guarantee poiner is uniquely owned
fn raw_do_unconditional_drop_payload_if_not_dropped<T: ?Sized, M: IMetadata>(
    item_ptr: *mut ItemHolder<T, M>,
    drop_job_queue: &mut impl IDropHandler<T, M>,
) {
    debug_println!("Unconditional drop of payload {:x?}", item_ptr);
    // SAFETY: Caller must guarantee pointer is uniquely owned
    let item = unsafe { &*(item_ptr) };
    let next_ptr = item.next.load(Ordering::SeqCst); //atomic drop check dropped
    let decoration = get_decoration(next_ptr);
    if !decoration.is_dropped() {
        debug_println!(
            "Actual drop of payload {:x?}, it wasn't already dropped",
            item_ptr
        );
        drop_job_queue.do_drop(item_ptr);
    }
}
fn do_dealloc<T: ?Sized, M: IMetadata>(item_ptr: *const ItemHolder<T, M>) {
    // SAFETY:
    // item_ptr is still a valid, exclusively owned pointer
    let layout = get_holder_layout(&unsafe { &*item_ptr }.payload);
    // SAFETY:
    // fullptr is a valid uniquely owned pointer that we must deallocate
    debug_println!("Calling dealloc {:x?}", item_ptr);

    #[cfg(feature = "validate")]
    {
        // SAFETY:
        // item_ptr is valid
        let item_ref = unsafe { &*(item_ptr as *mut ItemHolder<T, M>) };
        item_ref.magic1.store(0xdeaddead, Ordering::Relaxed);
        item_ref.magic2.store(0xdeaddea2, Ordering::Relaxed);
    }

    // SAFETY:
    // item_ptr is still a valid, exclusively owned pointer
    unsafe { alloc::alloc::dealloc(item_ptr as *mut _, layout) }
}

fn do_drop_strong<T: ?Sized, M: IMetadata>(
    full_item_ptr: *const ItemHolder<T, M>,
    drop_job_queue: &mut impl IDropHandler<T, M>,
) {
    debug_println!("drop strong of {:x?}", full_item_ptr,);
    let mut item_ptr = to_dummy(full_item_ptr);

    item_ptr = do_advance_strong::<T, M>(item_ptr, drop_job_queue);
    // SAFETY: do_advance_strong always returns a valid pointer
    let item: &ItemHolder<T, M> = unsafe { &*from_dummy(item_ptr) };

    let prior_strong = item.strong_count.fetch_sub(1, Ordering::SeqCst); //atomic drop dec strong_count
    debug_println!(
        "drop strong of {:x?}, prev count: {}, now {}",
        item_ptr,
        prior_strong,
        prior_strong - 1,
    );
    if prior_strong == 1 {
        // This is the only case where we actually might need to drop the rightmost node's payload
        // (if all nodes have only weak references)
        do_drop_weak::<T, M>(item_ptr, drop_job_queue);
    }
}

impl<T: ?Sized> Drop for ArcShift<T> {
    fn drop(&mut self) {
        verify_item(self.item.as_ptr());
        debug_println!("executing ArcShift::drop({:x?})", self.item.as_ptr());
        with_holder!(self.item, T, item, {
            let mut jobq = DropHandler::default();
            do_drop_strong(item, &mut jobq);
            jobq.resume_any_panics();
        })
    }
}
impl<T: ?Sized> Drop for ArcShiftWeak<T> {
    fn drop(&mut self) {
        verify_item(self.item.as_ptr());

        fn drop_weak_helper<T: ?Sized, M: IMetadata>(item: *const ItemHolder<T, M>) {
            let mut jobq = DropHandler::default();
            do_drop_weak::<T, M>(to_dummy(item), &mut jobq);
            jobq.resume_any_panics();
        }

        with_holder!(self.item, T, item, drop_weak_helper(item))
    }
}

/// This is a marker for methods that have been removed in the
/// most recent version of ArcShift.
///
/// Any methods that take this type as parameters, is completely
/// impossible to call. This is by design. The methods have been
/// kept, to hopefully make migration easier.
pub struct NoLongerAvailableMarker {
    _priv: (),
}

impl<T: ?Sized> ArcShiftWeak<T> {
    /// Upgrade this ArcShiftWeak instance to a full ArcShift.
    /// This is required to be able to access any stored value.
    /// If the ArcShift instance has no value (because the last ArcShift instance
    /// had been deallocated), this method returns None.
    pub fn upgrade(&self) -> Option<ArcShift<T>> {
        let t = with_holder!(self.item, T, item, {
            let mut drop_handler = DropHandler::default();
            let temp = do_upgrade_weak(item, &mut drop_handler);
            drop_handler.resume_any_panics();
            temp
        });
        Some(ArcShift {
            // SAFETY:
            // do_upgrade_weak returns a valid upgraded pointer
            item: unsafe { NonNull::new_unchecked(t? as *mut _) },
            pd: PhantomData,
        })
    }
}
impl<T> ArcShift<T> {
    /// Crate a new ArcShift instance with the given value.
    pub fn new(val: T) -> ArcShift<T> {
        let holder = make_sized_holder(val, null_mut());
        ArcShift {
            // SAFETY:
            // The newly created holder-pointer is valid and non-null
            item: unsafe { NonNull::new_unchecked(to_dummy(holder) as *mut _) },
            pd: PhantomData,
        }
    }

    /// Drops this ArcShift instance. If this was the last instance of the entire chain,
    /// the payload value is returned.
    pub fn try_into_inner(self) -> Option<T> {
        let mut drop_handler =
            deferred_panics_helper::stealing_drop::StealingDropHandler::default();
        verify_item(self.item.as_ptr());
        debug_println!("executing ArcShift::into_inner({:x?})", self.item.as_ptr());
        do_drop_strong(
            from_dummy::<T, SizedMetadata>(self.item.as_ptr()),
            &mut drop_handler,
        );
        core::mem::forget(self);
        drop_handler.take_stolen()
    }

    /// Update the value of this ArcShift instance (and all derived from it) to the given value.
    ///
    /// Note, other ArcShift instances will not see the new value until they call the
    /// [`ArcShift::get`] or [`ArcShift::reload`] methods.
    pub fn update(&mut self, val: T) {
        let holder = make_sized_holder(val, self.item.as_ptr() as *const ItemHolderDummy<T>);

        with_holder!(self.item, T, item, {
            let mut jobq = DropHandler::default();
            // SAFETY:
            // * do_update returns non-null values.
            // * 'holder' is in fact a thin pointer, just what we need for T, since T is Sized.
            let new_item = unsafe {
                NonNull::new_unchecked(
                    do_update(item, |_| Some(holder as *const _), &mut jobq) as *mut _
                )
            };
            jobq.resume_any_panics();
            self.item = new_item;
        });
    }
    /// Atomically update the value.
    ///
    /// The supplied closure 'update' is called with the previous value as an argument.
    /// It then constructs a new, updated value. This value replaces the previous value
    /// of the ArcShift instance. Note, if other threads are updating the arcshift chain
    /// concurrently, it may take multiple retries for the update to succeed. The closure
    /// will be called once for every attempt, until an attempt is successful. This means that
    /// the final updated value will have always been calculated from the previous set value.
    pub fn rcu(&mut self, mut update: impl FnMut(&T) -> T) -> &T {
        self.rcu_maybe(|x| Some(update(x)));
        (*self).shared_non_reloading_get()
    }

    /// Atomically update the value.
    ///
    /// The supplied closure 'update' is called with the previous value as an argument.
    /// It then constructs a new, updated value. This value replaces the previous value
    /// of the ArcShift instance. Note, if other threads are updating the arcshift chain
    /// concurrently, it may take multiple retries for the update to succeed. The closure
    /// will be called once for every attempt, until an attempt is successful. This means that
    /// the final updated value will have always been calculated from the previous set value.
    ///
    /// If the closure returns None, the update is cancelled, and this method returns false.
    pub fn rcu_maybe(&mut self, mut update: impl FnMut(&T) -> Option<T>) -> bool {
        let mut holder: Option<*const ItemHolderDummy<T>> = None;

        let mut cancelled = false;

        let mut jobq = DropHandler::default();
        // SAFETY:
        // do_update returns a valid non-null pointer
        let new_item_ptr = do_update(
            from_dummy::<T, SizedMetadata>(self.item.as_ptr()),
            |prev: &ItemHolder<T, SizedMetadata>| -> Option<*const ItemHolderDummy<T>> {
                // SAFETY:
                // we hold a strong ref on prev, so it cannot be dropped. Shared
                // access to 'payload' is thus sound.
                let prev_payload: &ManuallyDrop<T> = unsafe { &*prev.payload.get() };
                let prev_payload: &T = prev_payload;
                let new_candidate: Option<T> = update(prev_payload);
                let Some(new_candidate) = new_candidate else {
                    // Ok, the closure decided to NOT do an update after all
                    // We must now free any previous allocated candidate (both payload and holder).
                    if let Some(holder) = holder {
                        let mut jobq = DropHandler::default();
                        let full_ptr = from_dummy::<T, SizedMetadata>(holder);
                        raw_do_unconditional_drop_payload_if_not_dropped(
                            full_ptr as *mut ItemHolder<T, SizedMetadata>,
                            &mut jobq,
                        );
                        do_dealloc(full_ptr);
                        jobq.resume_any_panics();
                    }
                    cancelled = true;
                    return None;
                };
                if let Some(holder) = holder {
                    // SAFETY:
                    // If we get called when we already have a holder, then this is a retry
                    // and that holder was never visible to any other thread. We still have
                    // unique access to it.
                    let holder_mut: &mut ItemHolder<T, SizedMetadata> =
                        unsafe { &mut *(from_dummy::<T, SizedMetadata>(holder) as *mut _) };
                    // SAFETY:
                    // See above
                    unsafe {
                        ManuallyDrop::drop(&mut *holder_mut.payload.get());
                    }
                    holder_mut.payload = UnsafeCell::new(ManuallyDrop::new(new_candidate));
                    Some(holder)
                } else {
                    let new_holder = to_dummy(make_sized_holder(
                        new_candidate,
                        self.item.as_ptr() as *const ItemHolderDummy<T>,
                    ));
                    holder = Some(new_holder);
                    Some(new_holder)
                }
            },
            &mut jobq,
        );
        // SAFETY:
        // new_item_ptr returned by do_update is never null
        let new_unique_ptr = unsafe { NonNull::new_unchecked(new_item_ptr as *mut _) };
        jobq.resume_any_panics();
        self.item = new_unique_ptr;
        !cancelled
    }
}

#[doc(hidden)]
#[deprecated = "ArcShiftLight has been removed. Please consider using ArcShiftWeak. It is similar, though not a direct replacement."]
pub struct ArcShiftLight {}

/// Return value of [`ArcShift::shared_get`]
pub enum SharedGetGuard<'a, T: ?Sized> {
    /// The pointer already referenced the most recent value
    Raw(&'a T),
    /// We had to advance, and to do this we unfortunately had to clone
    LightCloned {
        #[doc(hidden)]
        next: *const ItemHolderDummy<T>,
    },
    /// Most expensive case, only used in some rare race-cases
    FullClone {
        #[doc(hidden)]
        cloned: ArcShift<T>,
    },
}

impl<T: ?Sized> Drop for SharedGetGuard<'_, T> {
    #[inline(always)]
    fn drop(&mut self) {
        match self {
            SharedGetGuard::Raw(_) => {}
            SharedGetGuard::FullClone { .. } => {}
            SharedGetGuard::LightCloned { next } => {
                if is_sized::<T>() {
                    let item_ptr = from_dummy::<_, SizedMetadata>(*next);
                    // SAFETY:
                    // The 'next' pointer of a SharedGetGuard::Cloned is always valid
                    let item = unsafe { &*item_ptr };
                    let mut dropq = DropHandler::default();
                    do_drop_strong(item, &mut dropq);
                    dropq.resume_any_panics();
                } else {
                    let item_ptr = from_dummy::<_, UnsizedMetadata<T>>(*next);
                    // SAFETY:
                    // The 'next' pointer of a SharedGetGuard::Cloned is always valid
                    let item = unsafe { &*item_ptr };
                    let mut dropq = DropHandler::default();
                    do_drop_strong(item, &mut dropq);
                    dropq.resume_any_panics();
                }
            }
        }
    }
}

impl<T: ?Sized> core::ops::Deref for SharedGetGuard<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        match self {
            SharedGetGuard::Raw(r) => r,
            SharedGetGuard::LightCloned { next } => {
                if is_sized::<T>() {
                    // SAFETY:
                    // The 'next' pointer of a SharedGetGuard::Cloned is always valid
                    unsafe { (*from_dummy::<_, SizedMetadata>(*next)).payload() }
                } else {
                    // SAFETY:
                    // The 'next' pointer of a SharedGetGuard::Cloned is always valid
                    unsafe { (*from_dummy::<_, UnsizedMetadata<T>>(*next)).payload() }
                }
            }
            SharedGetGuard::FullClone { cloned } => cloned.shared_non_reloading_get(),
        }
    }
}

fn slow_shared_get<T: ?Sized, M: IMetadata>(
    item: &ItemHolder<T, M>,
) -> Option<SharedGetGuard<'_, T>> {
    debug_println!("slow_shared_get: {:?} (advancing count)", item as *const _);
    item.advance_count.fetch_add(1, Ordering::SeqCst);

    #[cfg(loom)]
    atomic::fence(Ordering::SeqCst);

    debug_println!("advanced count");
    let next = item.next.load(Ordering::SeqCst);
    assert!(!undecorate(next).is_null());
    debug_println!("slow_shared_get: {:?}, next = {:?}", item as *const _, next);

    let next_val = from_dummy::<_, SizedMetadata>(undecorate(next));
    // SAFETY:
    // The 'item.next' pointer is not null as per method precondition, and since we have
    // incremented advance_count, 'item.next' is a valid pointer.
    let sc = unsafe { (*next_val).strong_count.load(Ordering::Relaxed) };
    if sc == 0 {
        debug_println!("slow_shared sc == 0");
        item.advance_count.fetch_sub(1, Ordering::SeqCst);
        return None;
    }
    debug_println!("slow_shared sc #1 for {:?}", next_val);
    // SAFETY:
    // The 'item.next' pointer is not null as per method precondition, and since we have
    // incremented advance_count, 'item.next' is a valid pointer.
    let exc = unsafe {
        (*next_val)
            .strong_count
            .compare_exchange(sc, sc + 1, Ordering::SeqCst, Ordering::SeqCst)
    };
    debug_println!("slow_shared sc #1.5");
    if exc.is_err() {
        debug_println!("slow_shared sc race");
        debug_println!("slow_shared_get: {:?}, sc == 0", item as *const _);
        item.advance_count.fetch_sub(1, Ordering::SeqCst);
        return None;
    }
    debug_println!("slow_shared sc #2");

    // SAFETY:
    // The 'item.next' pointer is not null as per method precondition, and since we have
    // incremented advance_count, 'item.next' is a valid pointer.
    let next_next = unsafe { (*next_val).next.load(Ordering::SeqCst) };
    if !undecorate(next_next).is_null() {
        debug_println!("slow_shared_get: {:?}, was dropped", item as *const _);
        let mut dropq = DropHandler::default();
        do_drop_strong(next_val, &mut dropq);
        item.advance_count.fetch_sub(1, Ordering::SeqCst);
        dropq.resume_any_panics();
        return None;
    }
    debug_println!("slow_shared sc #3");
    item.advance_count.fetch_sub(1, Ordering::SeqCst);

    debug_println!(
        "slow_shared_get: {:?}, advanced to {:?}",
        item as *const _,
        next
    );

    Some(SharedGetGuard::LightCloned {
        next: undecorate(next),
    })
}

impl<T: ?Sized> ArcShift<T> {
    #[doc(hidden)]
    #[cfg_attr(test, mutants::skip)]
    #[deprecated = "rcu_project was completely removed in ArcShift 0.2. Please use method 'rcu' instead, and just manually access the desired field."]
    pub fn rcu_project(&mut self, _marker: NoLongerAvailableMarker) {
        unreachable!("method cannot be called")
    }
    #[doc(hidden)]
    #[cfg_attr(test, mutants::skip)]
    #[deprecated = "rcu_maybe2 was completely removed in ArcShift 0.2. Please use 'rcu_maybe' instead. It has the same features."]
    pub fn rcu_maybe2(&mut self, _marker: NoLongerAvailableMarker) {
        unreachable!("method cannot be called")
    }

    #[doc(hidden)]
    #[cfg_attr(test, mutants::skip)]
    #[deprecated = "update_shared_box was completely removed in ArcShift 0.2. Shared references can no longer be updated. Please file an issue if this is a blocker!"]
    pub fn update_shared_box(&mut self, _marker: NoLongerAvailableMarker) {
        unreachable!("method cannot be called")
    }
    #[doc(hidden)]
    #[cfg_attr(test, mutants::skip)]
    #[deprecated = "update_shared was completely removed in ArcShift 0.2. Shared references can no longer be updated. Please file an issue if this is a blocker!"]
    pub fn update_shared(&mut self, _marker: NoLongerAvailableMarker) {
        unreachable!("method cannot be called")
    }
    #[doc(hidden)]
    #[cfg_attr(test, mutants::skip)]
    #[deprecated = "make_light was completely removed in ArcShift 0.2. Please use ArcShift::downgrade(&item) instead."]
    pub fn make_light(&mut self, _marker: NoLongerAvailableMarker) {
        unreachable!("method cannot be called")
    }

    /// Basically the same as doing [`ArcShift::new`], but avoids copying the contents of 'input'
    /// to the stack, even as a temporary variable. This can be useful, if the type is too large
    /// to fit on the stack.
    pub fn from_box(input: Box<T>) -> ArcShift<T> {
        let holder = make_sized_or_unsized_holder_from_box(input, null_mut());
        ArcShift {
            // SAFETY:
            // from_box_impl never creates a null-pointer from a Box.
            item: unsafe { NonNull::new_unchecked(holder as *mut _) },
            pd: PhantomData,
        }
    }

    /// Returns a mutable reference to the inner value.
    ///
    /// This method returns None if there are any other ArcShift or ArcShiftWeak instances
    /// pointing to the same object chain.
    pub fn try_get_mut(&mut self) -> Option<&mut T> {
        self.reload();
        with_holder!(self.item, T, item_ptr, {
            // SAFETY:
            // self.item is always a valid pointer for shared access
            let item = unsafe { &*item_ptr };
            let weak_count = item.weak_count.load(Ordering::SeqCst); //atomic try_get_mut check weak_count
            if get_weak_count(weak_count) == 1
                && !get_weak_prev(weak_count)
                // There can't be a 'next', since if so we wouldn't have a weak count of 1
                && item.strong_count.load(Ordering::SeqCst) == 1
            //atomic try_get_mut check strong count
            {
                // Yes, we need to check this again, see comment further down.
                let weak_count = item.weak_count.load(Ordering::SeqCst); //atomic try_get_mut double check weak_count
                if get_weak_count(weak_count) == 1 {
                    // SAFETY:
                    // The above constraints actually guarantee there can be no concurrent access.
                    //
                    // If we can prove that no such thread existed at time of 'weak_count.load' above,
                    // no such thread can later appear, since we hold '&mut self' to the only
                    // ArcShift referencing the chain. I.e, if we can prove this, we're done.
                    // No other weak-ref can exist, since we've checked that weak count is 1.
                    //
                    // Since the weak ref flags say there's no next and no prev, the only possible other
                    // ref that could exist at 'weak_count.load'-time would be a strong ref.
                    //
                    // We therefore check the strong count. If it is 1, there's only three possible cases:
                    // 1: We truly are the only remaining ref
                    // 2: There was one, and it has since gone away
                    // 3: There was one, and it has updated the chain and added a new value.
                    // 4: There was one, and it has downgraded to a weak ref.
                    //
                    // Cases 1 and 2 are benign. We guard against case 3 by loading 'weak_count'
                    // again. If it still shows there's no 'next', then case 3 is off the table.
                    // If the count is also still 1, option 4 is off the table. Meaning, we truly
                    // are alone.
                    let temp: &mut ManuallyDrop<T> = unsafe { &mut *item.payload.get() };
                    Some(&mut **temp)
                } else {
                    None
                }
            } else {
                None
            }
        })
    }

    /// See [`ArcShift::update`] .
    ///
    /// This method allows converting from [`Box<T>`]. This can be useful since it means it's
    /// possible to use unsized types, such as [`str`] or [`[u8]`].
    pub fn update_box(&mut self, new_payload: Box<T>) {
        let holder = make_sized_or_unsized_holder_from_box(new_payload, self.item.as_ptr());

        with_holder!(self.item, T, item, {
            let mut jobq = DropHandler::default();
            // SAFETY:
            // do_update returns a valid non-null pointer
            let new_item = unsafe {
                NonNull::new_unchecked(do_update(item, |_| Some(holder), &mut jobq) as *mut _)
            };
            jobq.resume_any_panics();
            self.item = new_item;
        });
    }

    /// Reload this ArcShift instance, and return the latest value.
    #[inline]
    pub fn get(&mut self) -> &T {
        if is_sized::<T>() {
            let ptr = from_dummy::<T, SizedMetadata>(self.item.as_ptr());
            // SAFETY:
            // self.item is always a valid pointer
            let item = unsafe { &*ptr };
            let next = item.next.load(Ordering::Relaxed);
            if !undecorate(next).is_null() {
                return Self::advance_strong_helper2::<SizedMetadata>(&mut self.item, true);
            }
            // SAFETY:
            // self.item is always a valid pointer
            unsafe { &*(item.payload.get() as *const T) }
        } else {
            let ptr = from_dummy::<T, UnsizedMetadata<T>>(self.item.as_ptr());
            // SAFETY:
            // self.item is always a valid pointer
            let item = unsafe { &*ptr };
            let next = item.next.load(Ordering::Relaxed);
            if !undecorate(next).is_null() {
                return Self::advance_strong_helper2::<UnsizedMetadata<T>>(&mut self.item, true);
            }
            // SAFETY:
            // self.item is always a valid pointer
            unsafe { item.payload() }
        }
    }

    /// Get the current value of this ArcShift instance.
    ///
    /// WARNING!
    /// Note, this method is fast if the ArcShift is already up to date. If a more recent
    /// value exists, this more recent value will be returned, but at a severe
    /// performance penalty (i.e, 'shared_get' on stale values is significantly slower than
    /// RwLock). This method is still lock-free.  The performance penalty will be even more
    /// severe if there is also contention.
    ///
    /// This method has the advantage that it doesn't require `&mut self` access,
    /// but is otherwise inferior to [`ArcShift::get`].
    #[inline(always)]
    pub fn shared_get(&self) -> SharedGetGuard<'_, T> {
        if is_sized::<T>() {
            let ptr = from_dummy::<T, SizedMetadata>(self.item.as_ptr());
            // SAFETY:
            // self.item is always a valid pointer
            let item = unsafe { &*ptr };
            let next = item.next.load(Ordering::Relaxed);
            if undecorate(next).is_null() {
                // SAFETY:
                // self.item is always a valid pointer
                return SharedGetGuard::Raw(unsafe { item.payload() });
            }

            slow_shared_get(item).unwrap_or_else(|| SharedGetGuard::FullClone {
                cloned: self.clone(),
            })
        } else {
            let ptr = from_dummy::<T, UnsizedMetadata<T>>(self.item.as_ptr());
            // SAFETY:
            // self.item is always a valid pointer
            let item = unsafe { &*ptr };
            let next = item.next.load(Ordering::Relaxed);
            if undecorate(next).is_null() {
                // SAFETY:
                // self.item is always a valid pointer
                return SharedGetGuard::Raw(unsafe { item.payload() });
            }

            slow_shared_get(item).unwrap_or_else(|| SharedGetGuard::FullClone {
                cloned: self.clone(),
            })
        }
    }

    /// Get the current value of this ArcShift instance.
    ///
    /// WARNING! This does not reload the pointer. Use [`ArcShift::reload()`] to reload
    /// the value, or [`ArcShift::get()`] to always get the newest value.
    ///
    /// This method has the advantage that it doesn't require `&mut self` access, and is very
    /// fast.
    #[inline(always)]
    pub fn shared_non_reloading_get(&self) -> &T {
        with_holder!(self.item, T, item, {
            // SAFETY:
            // ArcShift has a strong ref, so payload must not have been dropped.
            unsafe { (*item).payload() }
        })
    }

    /// Create an [`ArcShiftWeak<T>`] instance.
    ///
    /// Weak pointers do not prevent the inner from being dropped.
    ///
    /// The returned pointer can later be upgraded back to a regular ArcShift instance, if
    /// there is at least one remaining strong reference (i.e, [`ArcShift<T>`] instance).
    #[must_use = "this returns a new `ArcShiftWeak` pointer, \
                  without modifying the original `ArcShift`"]
    pub fn downgrade(this: &ArcShift<T>) -> ArcShiftWeak<T> {
        let t = with_holder!(this.item, T, item, do_clone_weak(item));
        ArcShiftWeak {
            // SAFETY:
            // do_clone_weak returns a valid, non-null pointer
            item: unsafe { NonNull::new_unchecked(t as *mut _) },
        }
    }

    /// Note, if there is at least one strong count, these collectively hold a weak count.
    #[allow(unused)]
    pub(crate) fn weak_count(&self) -> usize {
        with_holder!(self.item, T, item, {
            // SAFETY:
            // self.item is a valid pointer
            unsafe { get_weak_count((*item).weak_count.load(Ordering::SeqCst)) }
        })
    }
    #[allow(unused)]
    pub(crate) fn strong_count(&self) -> usize {
        with_holder!(self.item, T, item, {
            // SAFETY:
            // self.item is a valid pointer
            unsafe { (*item).strong_count.load(Ordering::SeqCst) }
        })
    }

    fn advance_strong_helper2<M: IMetadata>(
        old_item_ptr: &mut NonNull<ItemHolderDummy<T>>,
        gc: bool,
    ) -> &T {
        let mut jobq = DropHandler::default();
        let mut item_ptr = old_item_ptr.as_ptr() as *const _;
        // Lock free. Only loops if disturb-flag was set, meaning other thread
        // has offloaded work on us, and it has made progress.
        loop {
            item_ptr = do_advance_strong::<T, M>(item_ptr, &mut jobq);

            if gc {
                let (need_rerun, _strong_refs) =
                    do_janitor_task(from_dummy::<T, M>(item_ptr), &mut jobq);
                if need_rerun {
                    debug_println!("Janitor {:?} was disturbed. Need rerun", item_ptr);
                    continue;
                }
            }
            jobq.resume_any_panics();
            // SAFETY:
            // pointer returned by do_advance_strong is always a valid pointer
            *old_item_ptr = unsafe { NonNull::new_unchecked(item_ptr as *mut _) };
            // SAFETY:
            // pointer returned by do_advance_strong is always a valid pointer
            let item = unsafe { &*from_dummy::<T, M>(item_ptr) };
            // SAFETY:
            // pointer returned by do_advance_strong is always a valid pointer
            return unsafe { item.payload() };
        }
    }

    /// Reload this instance, making it point to the most recent value.
    #[inline(always)]
    pub fn reload(&mut self) {
        if is_sized::<T>() {
            let ptr = from_dummy::<T, SizedMetadata>(self.item.as_ptr());
            // SAFETY:
            // self.item is always a valid pointer
            let item = unsafe { &*ptr };
            let next = item.next.load(Ordering::Relaxed);
            if next.is_null() {
                return;
            }
            Self::advance_strong_helper2::<SizedMetadata>(&mut self.item, true);
        } else {
            let ptr = from_dummy::<T, UnsizedMetadata<T>>(self.item.as_ptr());
            // SAFETY:
            // self.item is always a valid pointer
            let item = unsafe { &*ptr };
            let next = item.next.load(Ordering::Relaxed);
            if next.is_null() {
                return;
            }
            Self::advance_strong_helper2::<UnsizedMetadata<T>>(&mut self.item, true);
        }
    }

    /// Check all links and refcounts.
    ///
    /// # SAFETY
    /// This method requires that no other threads access the chain while it is running.
    /// It is up to the caller to actually ensure this.
    #[allow(unused)]
    #[cfg(test)]
    #[cfg(feature = "std")]
    pub(crate) unsafe fn debug_validate(
        strong_handles: &[&Self],
        weak_handles: &[&ArcShiftWeak<T>],
    ) {
        let first = if !strong_handles.is_empty() {
            &strong_handles[0].item
        } else {
            &weak_handles[0].item
        };
        with_holder!(first, T, item, {
            Self::debug_validate_impl(strong_handles, weak_handles, item);
        });
    }
    #[allow(unused)]
    #[cfg(test)]
    #[cfg(feature = "std")]
    fn debug_validate_impl<M: IMetadata>(
        strong_handles: &[&ArcShift<T>],
        weak_handles: &[&ArcShiftWeak<T>],
        prototype: *const ItemHolder<T, M>,
    ) {
        let mut last = prototype;

        debug_println!("Start traverse right at {:?}", last);
        // Lock free - this is a debug method not used in production.
        loop {
            debug_println!("loop traverse right at {:?}", last);
            // SAFETY:
            // Caller guarantees that initial 'last' prototype is a valid pointer.
            // Other pointers are found by walking right from that.
            // Caller guarantees there are no concurrent updates.
            let next = unsafe { undecorate((*last).next.load(Ordering::SeqCst)) };
            debug_println!("Traversed to {:?}", next);
            if next.is_null() {
                break;
            }
            last = from_dummy::<T, M>(next);
        }

        let mut true_weak_refs =
            std::collections::HashMap::<*const ItemHolderDummy<T>, usize>::new();
        let mut true_strong_refs =
            std::collections::HashMap::<*const ItemHolderDummy<T>, usize>::new();
        // SAFETY:
        // the loop above has yielded a valid last ptr
        let all_nodes = unsafe { (*last).debug_all_to_left() };

        let strong_refs_exist = all_nodes
            .iter()
            .any(|x| x.strong_count.load(Ordering::SeqCst) > 0);

        if strong_refs_exist {
            debug_println!("Reading {:?}.next", to_dummy(last));
            // SAFETY:
            // the loop above has yielded a valid last ptr
            let last_next = unsafe { (*last).next.load(Ordering::SeqCst) };
            debug_println!("Reading {:?}.next gave {:?}", to_dummy(last), last_next);
            assert!(!get_decoration(last_next).is_dropped(), "the rightmost node must never be dropped (at least at rest, but regardless of refcount)");
        }

        for _node in all_nodes.iter() {
            debug_println!("Node: {:?}", *_node as *const ItemHolder<T, _>);
        }
        for node in all_nodes.iter().copied() {
            let next = undecorate(node.next.load(Ordering::SeqCst));
            if !next.is_null() {
                // SAFETY:
                // next pointer is valid
                assert!(all_nodes.contains(unsafe { &&*from_dummy::<T, M>(next) }));
            }
            let prev: *const _ = node.prev.load(Ordering::SeqCst);
            if !prev.is_null() {
                *true_weak_refs.entry(prev).or_default() += 1;
                // SAFETY:
                // prev pointer is valid
                assert!(all_nodes.contains(unsafe { &&*from_dummy::<T, M>(prev) }));
            }
            let advance_count = node.advance_count.load(Ordering::SeqCst);
            assert_eq!(advance_count, 0, "advance_count must always be 0 at rest");
        }

        for handle in weak_handles.iter() {
            // SAFETY:
            // All ArcShiftWeak instances always have valid, non-null pointers
            assert!(all_nodes.contains(unsafe { &&*from_dummy::<T, M>(handle.item.as_ptr()) }));
            let true_weak_count = true_weak_refs.entry(handle.item.as_ptr()).or_default();
            *true_weak_count += 1;
        }
        for handle in strong_handles.iter() {
            // SAFETY:
            // All ArcShift instances always have valid, non-null pointers
            assert!(all_nodes.contains(unsafe { &&*from_dummy::<T, M>(handle.item.as_ptr()) }));
            // SAFETY:
            // All ArcShift instances always have valid, non-null pointers
            let handle_next: *const _ = unsafe {
                (*from_dummy::<T, M>(handle.item.as_ptr()))
                    .next
                    .load(Ordering::SeqCst)
            };
            assert!(
                !get_decoration(handle_next).is_dropped(),
                "nodes referenced by strong handles must not be dropped, but {:?} was",
                handle.item.as_ptr()
            );

            let true_strong_count = true_strong_refs.entry(handle.item.as_ptr()).or_default();
            if *true_strong_count == 0 {
                *true_weak_refs.entry(handle.item.as_ptr()).or_default() += 1;
            }
            *true_strong_count += 1;
        }
        for node in all_nodes.iter().copied() {
            let node_dummy = to_dummy(node);
            let strong_count = node.strong_count.load(Ordering::SeqCst);
            let weak_count = get_weak_count(node.weak_count.load(Ordering::SeqCst));
            let expected_strong_count = true_strong_refs.get(&node_dummy).unwrap_or(&0);
            let expected_weak_count = true_weak_refs.get(&node_dummy).unwrap_or(&0);
            assert_eq!(
                strong_count, *expected_strong_count,
                "strong count of {:x?} should be {}",
                node as *const _, *expected_strong_count
            );
            assert_eq!(
                weak_count, *expected_weak_count,
                "weak count of {:x?} should be {}",
                node as *const _, *expected_weak_count
            );

            let next = node.next.load(Ordering::SeqCst);
            if strong_count > 0 {
                assert!(
                    !get_decoration(next).is_dropped(),
                    "as strong count is >0, node {:x?} should not be dropped",
                    node as *const _
                );
            }
        }
    }
}

#[cfg(test)]
#[cfg(not(any(loom, feature = "shuttle")))]
pub(crate) mod no_std_tests;

// Module for tests
#[cfg(all(test, feature = "std"))]
pub(crate) mod tests;
