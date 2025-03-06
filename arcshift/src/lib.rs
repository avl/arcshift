#![cfg_attr(feature = "nightly", feature(ptr_metadata))]
//TODO: Deny warnings! and stuff again!
//#![deny(warnings)]
//#![forbid(clippy::undocumented_unsafe_blocks)]
//#![deny(missing_docs)]

//! # Introduction to ArcShift
//!
//! [`ArcShift`] is a data type similar to [`std::sync::Arc`], except that it allows updating
//! the value pointed to. The memory overhead is identical to that of Arc. ArcShift is mainly
//! intended for cases where updates are very infrequent. See the 'Limitations'-heading
//! further down before using!
//!
//! ## Example
//! ```rust
//! # #[cfg(not(any(loom,feature="shuttle")))]
//! # {
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
//!     println!("Value in thread 1: '{}'", *arc); //Prints 'Hello'
//!     arc.update("New value".to_string());
//!     println!("Updated value in thread 1: '{}'", *arc); //Prints 'New value'
//! });
//!
//! let j2 = thread::spawn(move||{
//!     // Prints either 'Hello' or 'New value', depending on scheduling:
//!     println!("Value in thread 2: '{}'", *arc2);
//! });
//!
//! j1.join().unwrap();
//! j2.join().unwrap();
//! # }
//! ```
//!
//! When ArcShift values are updated, a linked list of all updates is formed. Whenever
//! an arcshift-instance is reloaded (using [`ArcShift::reload`], [`ArcShift::get`] or
//! [`ArcShiftWeak::reload`], that instance advances along the linked list to the last
//! node in the list. When no instance exists pointing at a node in the list, it is dropped.
//! It is thus important to periodically call reload or get (unless the number of updates is
//! so low that the cost of traversing the linked list is acceptable).
//!
//! # Strong points
//! * Easy to use (similar to Arc)
//! * All functions are lock free (see <https://en.wikipedia.org/wiki/Non-blocking_algorithm> )
//! * For use cases where no modification of values occurs, performance is very good (much
//!   better than RwLock or Mutex).
//! * Modifying values is reasonably fast (think, 10-50 nanoseconds).
//! * The function [`ArcShift::shared_non_reloading_get`] allows access almost without any overhead
//!   at all compared to regular Arc.
//! * ArcShift does not rely on any thread-local variables to achieve its performance.
//!
//! # Limitations
//!
//! ArcShift achieves its performance at the expense of the following disadvantages:
//!
//! * When modifying the value, the old version of the value lingers in memory until
//!   the last ArcShift has been updated. Such an update only happens when the ArcShift
//!   is accessed using a unique (`&mut`) access (like [`ArcShift::get`] or [`ArcShift::reload`]).
//!   This can be partially mitigated by using the [`ArcShiftWeak`]-type for long-lived
//!   never-reloaded instances.
//! * Modifying the value is approximately 10x more expensive than modifying `Arc<RwLock<T>>`
//! * When the value is modified, the next subsequent access can be slower than an `Arc<RwLock<T>>`
//!   access.
//! * ArcShift is its own datatype. It is in no way compatible with `Arc<T>`.
//! * At most 524287 instances of ArcShiftWeak can be created for each value.
//! * At most 35000000000000 instances of ArcShift can be created for each value.
//! * ArcShift does not support an analog to [`std::sync::Arc`]'s [`std::sync::Weak`].
//! * ArcShift instances should ideally be owned (or be mutably accessible).
//!
//! The last limitation might seem unacceptable, but for many applications it is not
//! hard to make sure each thread/scope has its own instance of ArcShift pointing to
//! the resource. Cloning ArcShift instances is reasonably fast.
//!
//! # Motivation
//!
//! The primary raison d'être for [`ArcShift`] is to be a version of Arc which allows
//! modifying the stored value, with very little overhead over regular Arc, as long as
//! updates are very infrequent.
//!
//! The motivating use-case for ArcShift is reloadable assets in computer games.
//! During normal usage, assets do not change. All benchmarks and play experience will
//! be dependent only on this baseline performance. Ideally, we therefore want to have
//! a very small performance penalty for the case when assets are *not* updated, compared
//! to using regular [`std::sync::Arc`].
//!
//! During game development, artists may update assets, and hot-reload is a very
//! time-saving feature. A performance hit during asset-reload is acceptable though.
//! ArcShift prioritizes base performance, while accepting a penalty when updates are made.
//! The penalty is that, under some circumstances described below, ArcShift can have a lingering
//! performance hit until 'reload' is called. See documentation for the details.
//!
//! ArcShift can, of course, be useful in other domains than computer games.
//!
//! # Performance properties
//!
//! Accessing the value stored in an ArcShift instance only requires a single
//! atomic operation, of the least expensive kind (Ordering::Relaxed). On x86_64,
//! this is the exact same machine operation as a regular memory access, and also
//! on arm it is not an expensive operation.
//! The cost of such access is much smaller than a mutex access, even an uncontended one.
//!
//!
//! # Implementation
//!
//! The basic idea of ArcShift is that each ArcShift instance points to a small heap block,
//! that contains the pointee value of type T, a reference count, and a 'next'-pointer. The
//! 'next'-pointer starts out as null, but when the value in an ArcShift is updated, the
//! 'next'-pointer is set to point to the updated value.
//!
//! This means that each ArcShift-instance always points at valid value of type T. No locking
//! or synchronization is required to get at this value. This is why ArcShift instances are fast
//! to use. But it has the drawback that as long as an ArcShift-instance exists, whatever value
//! it points to must be kept alive. Each time an ArcShift instance is accessed mutably, we have
//! an opportunity to update its pointer to the 'next' value. When the last ArcShift-instance
//! releases a particular value, it will be dropped. The operation to update the pointer is called
//! a 'reload'.
//!
//! ArcShiftWeak-instances also keep pointers to the heap blocks mentioned above, but value T
//! in the block can be dropped while being held by an ArcShiftWeak. This means that ArcShiftWeak-
//! instances only consume `std::mem::size_of::<T>()` bytes of memory, when the value they
//! point to has been dropped. When the ArcShiftWeak-instances is reloaded, or dropped, that memory
//! is also released.
//!
//!
//! # Pitfall #1 - lingering memory usage
//!
//! Be aware that ArcShift instances that are just "lying around" without ever being reloaded,
//! will keep old values around, taking up memory. This is a fundamental drawback of the approach
//! taken by ArcShift. One workaround is to replace long-lived non-reloaded instances of
//! [`ArcShift`] with [`ArcShiftWeak`]. This alleviates the problem.
//!
//! # Pitfall #2 - reference count limitations
//!
//! ArcShift uses a single 64 bit reference counter to track both ArcShift
//! and ArcShiftWeak instance counts. This is achieved by giving each ArcShiftWeak-instance a
//! weight of 1, while each ArcShift-instance receives a weight of 524288. As a consequence
//! of this, the maximum number of ArcShiftWeak-instances (for the same value), is 524287.
//! Because the counter is 64-bit, this leaves 2^64/524288 as the maximum
//! number of ArcShift instances (for the same value). However, we leave some margin, to allow
//! practically detecting any overflow, giving a maximum of 35000000000000,
//! Since each ArcShift instance takes at least 8 bytes of space, it takes at least 280TB of memory
//! to even be able to hit this limit. If the limit is somehow reached, there will be a best effort
//! at detecting this and causing a panic. This is similar to how the rust std library handles overflow
//! of the reference counter on std::sync::Arc. Just as with std::core::Arc, the overflow
//! will be detected in practice, though there is no guarantee. For ArcShift, the overflow will be
//! detected as long as the machine has an even remotely fair scheduler, and less than 100 billion
//! threads (though the conditions for detection of std::core::Arc-overflow are even more assured).
//!
//!
//! # A larger example
//!
//! ```rust
//! # #[cfg(not(any(loom,feature="shuttle")))]
//! # {
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
//! # }
//! ```
//!

/*
TODO: Remove this section.

Talking points:

Arc<[u8]> is actually 2 words (16 bytes on 64-bit platforms).

This works, because the length can't be changed. But for ArcShift<[u8]>, we'd expect to be able
to change the length!

When decreasing a reference count, if it doesn't go to 0, you know _nothing_ about the count.

 */

use std::alloc::Layout;
#[allow(unused)]
use std::backtrace::Backtrace;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::marker::PhantomData;
use std::mem::{transmute, ManuallyDrop};
use std::ops::Deref;
use std::panic::UnwindSafe;
use std::ptr::{addr_of_mut, null, null_mut, NonNull};
use std::sync::atomic::Ordering;
use std::task::Context;
// About unsafe code in this crate:
// Some private functions contain unsafe code, and place limitations on their
// callers, without these private functions being marked unsafe.
// The rationale is that there are lots of operations that simply aren't unsafe, like
// assigning null to a pointer, that could cause UB in unsafe code in this crate.
// This crate is inherently dependent on all the code in it being correct. Therefore,
// marking more functions unsafe buys us very little.
// Note! The API of this crate is 100% safe and it should be impossible to trigger UB through it.

// All atomic primitives are reexported from a
// local module called 'atomic', so we can easily change between using
// types from 'std' (normal case) and types from shuttle/loom testing libraries.

/// Declarations of atomic ops for using Arcshift in production
#[cfg(all(not(loom), not(feature = "shuttle")))]
mod atomic {
    //pub use std::hint::spin_loop;
    pub use std::sync::atomic::{fence, AtomicPtr, AtomicUsize, Ordering};
    #[allow(unused)]
    pub use std::thread;
}

/// Declarations for verifying Arcshift using 'shuttle'
#[cfg(feature = "shuttle")]
mod atomic {
    pub use shuttle::hint::spin_loop;
    pub use shuttle::sync::atomic::{fence, AtomicPtr, AtomicUsize, Ordering};
    #[allow(unused)]
    pub use shuttle::thread;
    #[allow(unused)]
    pub use std::sync::atomic::AtomicU64;
}

/// Declarations for verifying Arcshift using 'loom'
#[cfg(loom)]
mod atomic {
    pub use loom::hint::spin_loop;
    pub use loom::sync::atomic::{fence, AtomicPtr, AtomicUsize, Ordering};
    #[allow(unused)]
    pub use loom::thread;
    #[allow(unused)]
    pub use std::sync::atomic::AtomicU64;
}

/// Define a macro for debug-output, only used in debug-builds.
#[cfg(all(feature = "debug", not(loom)))]
macro_rules! debug_println {
    ($($x:tt)*) => {
        println!("{:?}: {}", crate::atomic::thread::current().id(), format!($($x)*))
    }
}
#[cfg(all(feature = "debug", loom))]
macro_rules! debug_println {
    ($($x:tt)*) => { println!($($x)*) }
}


#[cfg(not(feature = "debug"))]
macro_rules! debug_println {
    ($($x:tt)*) => {{}};
}

/// Smart pointer with similar use case as std::sync::Arc, but with
/// the added ability to atomically replace the contents of the Arc.
/// See `crate` documentation for more information.
///
/// ```rust
/// # #[cfg(not(any(loom,feature="shuttle")))]
/// # {
/// # extern crate arcshift;
/// # use arcshift::ArcShift;
/// let instance = ArcShift::new("test");
/// println!("Value: {:?}", *instance);
/// # }
/// ```
pub struct ArcShift<T: ?Sized> {
    item: NonNull<ItemHolderDummy<T>>,

}
impl<T> UnwindSafe for ArcShift<T> {}

impl<T: ?Sized> Debug for ArcShift<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ArcShift(todo!)")
    }
}
impl<T: ?Sized> Debug for ArcShiftWeak<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ArcShiftWeak(..)")
    }
}


/// ArcShiftWeak is like ArcShift, except it does not provide overhead-free access.
/// However, it has the advantage of not preventing old versions of the payload type from being
/// freed.
///
/// TODO: Reenable this test
/// ```ignore
/// # #[cfg(not(any(loom,feature="shuttle")))]
/// # {
/// # extern crate arcshift;
/// # use arcshift::ArcShiftWeak;
/// let light_instance = ArcShiftWeak::new("test");
/// let instance = light_instance.upgrade();
/// println!("Value: {:?}", *instance);
/// # }
/// ```
///
/// WARNING! Because of implementation reasons, each instance of ArcShiftWeak will claim
/// a memory equal to `size_of::<T>` (plus a bit), even if the value inside it has been moved out,
/// and even if all other instances of ArcShift/ArcShiftWeak have been dropped.
/// If this limitation is unacceptable, consider using `ArcShiftWeak<Box<T>>` as your datatype,
/// or possibly using a different crate.
pub struct ArcShiftWeak<T: ?Sized> {
    item: NonNull<ItemHolderDummy<T>>,
}

/// A handle that allows reloading an ArcShift instance without having 'mut' access.
/// However, it does not implement `Sync`.
pub mod cell;

const fn is_sized<T: ?Sized>() -> bool {
    size_of::<&T>() == size_of::<&()>()
}


/// SAFETY:
/// If `T` is `Sync`, `ArcShift<T>` can also be `Sync`
unsafe impl<T: Sync + ?Sized> Sync for ArcShift<T> {}

/// SAFETY:
/// If `T` is `Send`, `ArcShift<T>` can also be `Send`
unsafe impl<T:  Send + ?Sized> Send for ArcShift<T> {}


/// SAFETY:
/// If `T` is `Sync`, `ArcShift<T>` can also be `Sync`
unsafe impl<T:  Sync + ?Sized> Sync for ArcShiftWeak<T> {}

/// SAFETY:
/// If `T` is `Send`, `ArcShift<T>` can also be `Send`
unsafe impl<T:  Send + ?Sized> Send for ArcShiftWeak<T> {}


/*fn make_sized_holder<T, M: IMetadata>(item: ItemHolder<T, M>) -> *mut ItemHolder<T,M> {
    let cur_ptr = Box::into_raw(Box::new(item));
    cur_ptr as _
}*/

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
impl<T: ?Sized> std::fmt::Debug for UnsizedMetadata<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
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
        std::mem::transmute_copy(&FatPtr {
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
fn arc_from_raw_parts<T: ?Sized, M: IMetadata>(
    data_ptr: *const u8,
    metadata: UnsizedMetadata<T>,
) -> *const ItemHolder<T, M> {
    // SAFETY:
    // This is the best I managed without using nightly-only features (August 2024).
    // It is sound as long as the actual internal representation of fat pointers doesn't change.
    #[cfg(not(feature = "nightly"))]
    unsafe {
        std::mem::transmute_copy(&FatPtr {
            ptr: data_ptr as *mut u8,
            meta: metadata,
        })
    }
    #[cfg(feature = "nightly")]
    {
        std::ptr::from_raw_parts(data_ptr, metadata.meta)
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

fn get_holder_layout<T: ?Sized >(ptr: *const T) -> Layout {
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

fn to_dummy<T:?Sized,M:IMetadata>(ptr: *const ItemHolder<T, M>) -> *const ItemHolderDummy<T> {
    ptr as *mut ItemHolderDummy<T>
}
fn from_dummy<T:?Sized,M:IMetadata>(ptr: *const ItemHolderDummy<T>) -> *const ItemHolder<T, M> {
    get_full_ptr_raw::<T,M>(ptr)
}

macro_rules! with_holder {
    ($p: expr, $t: ty, $f:expr) => {
        if is_sized::<$t>() {
            let ptr = from_dummy::<$t, SizedMetadata>($p.as_ptr());
            $f(ptr)
        } else {
            let ptr = from_dummy::<$t, UnsizedMetadata<$t>>($p.as_ptr());
            $f(ptr)
        }
    };
}

fn make_sized_or_unsized_holder_from_box<T: ?Sized>(item: Box<T>, prev: *mut ItemHolderDummy<T>) -> *const ItemHolderDummy<T> {
    if is_sized::<T>() {
        to_dummy(make_sized_holder_from_box(item, prev))
    } else {
        to_dummy(make_unsized_holder_from_box(item, prev))
    }

}
#[allow(unused)]
fn make_unsized_holder_from_box<T: ?Sized>(item: Box<T>, prev: *mut ItemHolderDummy<T>) -> *const ItemHolder<T, UnsizedMetadata<T>> {
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
            // std::alloc::alloc requires the allocated layout to have a nonzero size. This
            // is fulfilled, since ItemHolder is non-zero sized even if T is zero-sized.
            // The returned memory is uninitialized, but we will initialize the required parts of it
            // below.
            unsafe { arc_from_raw_parts_mut(std::alloc::alloc(layout) as *mut _, metadata) };
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
    }    // SAFETY:
    // weak_count is just an AtomicUsize-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).weak_count).write(atomic::AtomicUsize::new(1));
    }
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
fn make_sized_holder<T>(payload: T, prev: *mut ItemHolder<T, SizedMetadata>) -> *mut ItemHolder<T, SizedMetadata> {

    let holder = ItemHolder {
        the_meta: SizedMetadata,
        #[cfg(feature = "validate")]
        magic1: std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111),
        next: Default::default(),
        prev: atomic::AtomicPtr::new(to_dummy::<T, SizedMetadata>(prev) as *mut _),
        strong_count: atomic::AtomicUsize::new(1),
        weak_count: atomic::AtomicUsize::new(1),
        advance_count: atomic::AtomicUsize::new(0),
        #[cfg(feature = "validate")]
        magic2: std::sync::atomic::AtomicU64::new(0x1234123412348111),
        payload: UnsafeCell::new(ManuallyDrop::new(payload)),
    };

    Box::into_raw(Box::new(holder))
}

#[allow(unused)]
fn make_sized_holder_from_box<T: ?Sized>(item: Box<T>, prev: *mut ItemHolderDummy<T>) -> *mut ItemHolder<T, SizedMetadata> {
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
        unsafe { std::mem::transmute_copy(&std::alloc::alloc(layout)) };

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
        addr_of_mut!((*item_holder_ptr).prev).write(atomic::AtomicPtr::new(prev as * mut _));
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
    }    // SAFETY:
    // weak_count is just an AtomicUsize-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).weak_count).write(atomic::AtomicUsize::new(1));
    }
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
fn get_ptr<T: ?Sized>(dummy: NonNull<ItemHolderDummy<T>>) -> *const ItemHolderDummy<T> {
    dummy.as_ptr() as *const ItemHolderDummy<T>
}
fn get_full_ptr<T: ?Sized, M: IMetadata>(
    dummy: NonNull<ItemHolderDummy<T>>,
) -> *const ItemHolder<T, M> {
    get_full_ptr_raw(dummy.as_ptr())
}
fn get_full_ptr_raw<T: ?Sized, M: IMetadata>(
    dummy: *const ItemHolderDummy<T>,
) -> *const ItemHolder<T, M> {
    if is_sized::<T>() {
        debug_assert_eq!(
            size_of::<*const ItemHolder<T, M>>(),
            size_of::<*const ItemHolderDummy<T>>()
        ); //<- Verify that *const T is not a fat ptr

        // SAFETY:
        // '*const ItemHolder<T, M>' is not _actually_ a fat pointer (it is just pointer sized),
        // so transmuting from dummy to it is correct.
        unsafe { std::mem::transmute_copy(&dummy) }
    } else {
        debug_assert_eq!(
            size_of::<M>(),
            size_of::<usize>()
        ); //<- Verify that *const ItemHolder<T> is a fat ptr

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


struct SizedMetadata;
trait IMetadata {}
impl IMetadata for SizedMetadata {}
impl<T: ?Sized> IMetadata for UnsizedMetadata<T> {}

#[repr(transparent)]
struct ItemHolderDummy<T:?Sized> {
    // void pointers should point to u8
    dummy: u8,
    phantom_data: PhantomData<T>
}

/// Align 4 is needed, since we store flags in the lower 2 bits of the ItemHolder-pointers
/// In practice, the alignment of ItemHolder is 8 anyway, but we specify it here for clarity.
#[repr(align(8))]
#[repr(C)] // Just to get the 'magic' first and last in memory. Shouldn't hurt.
struct ItemHolder<T: ?Sized, M: IMetadata> {
    the_meta: M,
    #[cfg(feature = "validate")]
    magic1: std::sync::atomic::AtomicU64,
    /// Pointer to the next value or null. Possibly decorated (i.e, least significant bit set)
    /// The decoration determines:
    ///  * If janitor process is currently active for this node and those left of it
    ///  * If payload has been deallocated
    next: atomic::AtomicPtr<ItemHolderDummy<T>>,
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
    weak_count: atomic::AtomicUsize,
    /// Can be incremented to keep the 'next' value alive. If 'next' is set to a new value, and
    /// 'advance_count' is read as 0 after, then this node is definitely not going to advance to
    /// some node *before* 'next'.
    advance_count: atomic::AtomicUsize,
    #[cfg(feature = "validate")]
    magic2: std::sync::atomic::AtomicU64,

    payload: UnsafeCell<ManuallyDrop<T>>, //TODO: We should use ManuallyDrop<T> here. It would simplify the code (but not affect correctness)
}

impl<'a,T:?Sized,M:IMetadata> PartialEq for ItemHolder<T,M> {
    fn eq(&self, other: &ItemHolder<T,M>) -> bool {
        std::ptr::addr_eq(self as *const _, other as *const _)
    }
}

impl<T: ?Sized, M: IMetadata> ItemHolder<T, M> {

    fn set_next(&self, undecorated_new_next: *const ItemHolderDummy<T>) {
        debug_assert_eq!(get_decoration(undecorated_new_next), ItemStateEnum::Undecorated);
        loop {
            let prior_next = self.next.load(atomic::Ordering::SeqCst);
            let new_next = decorate(undecorated_new_next, get_decoration(prior_next));
            if self.next.compare_exchange(prior_next, new_next as *mut _, atomic::Ordering::SeqCst, atomic::Ordering::SeqCst).is_ok() {
                debug_println!("set {:x?}.next = {:x?}", to_dummy(self as *const _), new_next);
                return;
            }
        }
    }


    /// Includes self
    fn debug_all_to_left<'a>(&self) -> Vec<&'a ItemHolder<T,M>> {
        let mut ret = vec![];
        let mut item_ptr = self as *const ItemHolder<T,M>;
        loop {
            ret.push(unsafe{&*item_ptr});
            let prev = from_dummy::<T,M>(unsafe{(*item_ptr).prev.load(Ordering::SeqCst)});
            if prev.is_null() {
                break;
            }
            item_ptr = prev;
        }

        ret
    }

    fn decoration(&self) -> ItemStateEnum {
        get_decoration(self.next.load(Ordering::SeqCst))
    }

    fn has_payload(&self)  -> bool {
        !get_decoration(self.next.load(atomic::Ordering::SeqCst)).is_dropped()
    }

    unsafe fn payload<'a>(&self) -> &'a T  {
        unsafe { transmute::<&T,&'a T>(&*(self.payload.get())) }
    }
    /// Returns true if the node could be atomically locked
    fn lock_node_for_gc(&self) -> bool {
        let cur_next = self.next.load(atomic::Ordering::SeqCst);
        let decoration = get_decoration(cur_next);
        match decoration {
            ItemStateEnum::Undecorated|ItemStateEnum::PayloadDropped => {
                let decorated = decorate(undecorate(cur_next), decoration.with_gc());
                let success = self.next.compare_exchange(cur_next, decorated as *mut _, Ordering::SeqCst, Ordering::SeqCst).is_ok();
                debug_println!("Locking node {:x?}, result: {} (prior decoration: {:?}, new {:?})", self as * const ItemHolder<T, M>, success, decoration, get_decoration(decorated));
                success
            }
            _ => {
                debug_println!("Locking node {:x?} failed, already decorated", self as * const ItemHolder<T, M>);
                // Already decorated
                false
            }
        }
    }
    fn unlock_node(&self) {
        debug_println!("Unlocking node {:x?}", self as * const ItemHolder<T, M>);
        loop {
            let cur_next = self.next.load(atomic::Ordering::SeqCst);
            let decoration = get_decoration(cur_next);
            let undecorated =
                decorate(undecorate(cur_next), decoration.without_gc());
            debug_println!("Unlocked dec: {:x?}", get_decoration(undecorated));
            if self.next.compare_exchange(cur_next, undecorated as *mut _, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                return;
            }
        }
    }
}

#[cfg(all(any(loom, feature = "shuttle"), feature = "validate"))]
static MAGIC: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);

impl<T: ?Sized, M: IMetadata> ItemHolder<T, M> {
    #[cfg_attr(test, mutants::skip)]
    #[cfg(feature = "validate")]
    fn verify(ptr2: *const ItemHolder<T, M>) {
        {
            assert_is_undecorated(ptr2);
            let ptr = ptr2 as *const ItemHolder<(), M>;

            // SAFETY:
            // This function is never called with a null-ptr. Also, this is
            // just used for testing.
            let atomic_magic1 = unsafe { &*std::ptr::addr_of!((*ptr).magic1) };
            // SAFETY:
            // This function is never called with a null-ptr. Also, this is
            // just used for testing.
            let atomic_magic2 = unsafe { &*std::ptr::addr_of!((*ptr).magic2) };

            let magic1 = atomic_magic1.load(Ordering::SeqCst);
            let magic2 = atomic_magic2.load(Ordering::SeqCst);
            if magic1 >> 16 != 0xbeefbeefbeef {
                eprintln!(
                    "Internal error - bad magic1 in {:?}: {} ({:x})",
                    ptr, magic1, magic1
                );
                debug_println!(
                    "Internal error - bad magic1 in {:?}: {} ({:x})",
                    ptr,
                    magic1,
                    magic1
                );
                debug_println!("Backtrace: {}", Backtrace::capture());
                panic!();
            }
            if magic2 >> 16 != 0x123412341234 {
                eprintln!(
                    "Internal error - bad magic2 in {:?}: {} ({:x})",
                    ptr, magic2, magic2
                );
                debug_println!(
                    "Internal error - bad magic2 in {:?}: {} ({:x})",
                    ptr,
                    magic2,
                    magic2
                );
                debug_println!("Backtrace: {}", Backtrace::capture());
                panic!();
            }
            #[cfg(not(any(loom, feature = "shuttle")))]
            {
                let m1 = magic1 & 0xffff;
                let m2 = magic2 & 0xffff;
                if m1 != 0x8111 || m2 != 0x8111 {
                    eprintln!("Internal error - bad magic in {:?} {:x} {:x}", ptr, m1, m2);
                }
            }

            #[cfg(any(loom, feature = "shuttle"))]
            {
                let diff = (magic1 & 0xffff) as isize - (magic2 & 0xffff) as isize;
                if diff != 0 {
                    eprintln!(
                        "Internal error - bad magics in {:?}: {} ({:x}) and {} ({:x})",
                        ptr, magic1, magic1, magic2, magic2
                    );
                    debug_println!(
                        "Internal error - bad magics in {:?}: {} ({:x}) and {} ({:x})",
                        ptr,
                        magic1,
                        magic1,
                        magic2,
                        magic2
                    );
                    //println!("Backtrace: {}", Backtrace::capture());
                    panic!();
                }
                let magic = MAGIC.fetch_add(1, Ordering::SeqCst);
                let magic = magic as i64 as u64;
                atomic_magic1.fetch_and(0xffff_ffff_ffff_0000, Ordering::SeqCst);
                atomic_magic2.fetch_and(0xffff_ffff_ffff_0000, Ordering::SeqCst);
                atomic_magic1.fetch_or(magic, Ordering::SeqCst);
                atomic_magic2.fetch_or(magic, Ordering::SeqCst);
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
        let ptr = _ptr;
        let x = undecorate(ptr);
        if x != ptr {
            panic!("Internal error in ArcShift: Pointer given to verify was decorated, it shouldn't have been! {:?} (={:?})", ptr, get_state(ptr));
        }
        if x.is_null() {
            return;
        }
        ItemHolder::<T, M>::verify(x)
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
#[cfg(feature = "validate")]
#[cfg_attr(test, mutants::skip)] // This is only used for validation and test, it has no behaviour
impl<T: ?Sized, M: IMetadata> Drop for ItemHolder<T, M> {
    fn drop(&mut self) {
        Self::verify(self as *mut _ as *mut ItemHolderDummy<T>);
        debug_println!("ItemHolder<T>::drop {:?}", self as *const ItemHolder<T, M>);
        {
            self.magic1 = std::sync::atomic::AtomicU64::new(0xDEADDEA1DEADDEA1);
            self.magic2 = std::sync::atomic::AtomicU64::new(0xDEADDEA2DEADDEA2);
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ItemStateEnum {
    /// Pointer is not decorated
    Undecorated = 0,
    GcIsActive = 1,
    PayloadDropped = 2,
    PayloadDroppedAndGcActive = 3,
}

impl ItemStateEnum {
    fn dropped(self) -> ItemStateEnum {
        match self {
            ItemStateEnum::Undecorated => {ItemStateEnum::PayloadDropped}
            ItemStateEnum::GcIsActive => {ItemStateEnum::PayloadDroppedAndGcActive}
            ItemStateEnum::PayloadDropped => {ItemStateEnum::PayloadDropped}
            ItemStateEnum::PayloadDroppedAndGcActive => {ItemStateEnum::PayloadDroppedAndGcActive}
        }
    }
    fn with_gc(self) -> ItemStateEnum {
        match self {
            ItemStateEnum::Undecorated => {ItemStateEnum::GcIsActive}
            ItemStateEnum::GcIsActive => {ItemStateEnum::GcIsActive}
            ItemStateEnum::PayloadDropped => {ItemStateEnum::PayloadDroppedAndGcActive}
            ItemStateEnum::PayloadDroppedAndGcActive => {ItemStateEnum::PayloadDroppedAndGcActive}
        }
    }
    fn without_gc(self) -> ItemStateEnum {
        match self {
            ItemStateEnum::Undecorated => {ItemStateEnum::Undecorated}
            ItemStateEnum::GcIsActive => {ItemStateEnum::Undecorated}
            ItemStateEnum::PayloadDropped => {ItemStateEnum::PayloadDropped}
            ItemStateEnum::PayloadDroppedAndGcActive => {ItemStateEnum::PayloadDropped}
        }
    }
    fn is_dropped(self) -> bool {
        match self {
            ItemStateEnum::Undecorated => {false}
            ItemStateEnum::GcIsActive => {false}
            ItemStateEnum::PayloadDropped => {true}
            ItemStateEnum::PayloadDroppedAndGcActive => {true}
        }
    }
}

//TODO: look at use of these, and make more optimized primitives
/// Decorate the given pointer with the enum value.
/// The decoration is the least significant 2 bits.
fn decorate<T: ?Sized>(
    ptr: *const ItemHolderDummy<T>,
    e: ItemStateEnum,
) -> *const ItemHolderDummy<T> {
    let curdecoration = (ptr as usize) & 3;
    ((ptr as *const u8).wrapping_offset((e as isize) - (curdecoration as isize)))
        as *const ItemHolderDummy<T>
}

/// Get the state encoded in the decoration, if any.
/// Returns None if the pointer is undecorated null.
/// The pointer must be valid.
fn get_decoration<T: ?Sized>(ptr: *const ItemHolderDummy<T>) -> ItemStateEnum {
    if ptr.is_null() {
        return ItemStateEnum::Undecorated;
    }
    let raw = ((ptr as usize) & 3) as u8;
    /*#[cfg(feature = "validate")]
    if raw == 0 {
        return ItemStateEnum::Undecorated;
    }*/
    // SAFETY:
    // All values `0..=3` are valid ItemStateEnum.
    // And the bitmask produces a value `0..=3`
    unsafe { std::mem::transmute::<u8, ItemStateEnum>(raw) }
}

/// Panic if the pointer is decorated
#[cfg_attr(test, mutants::skip)]
#[inline]
fn assert_is_undecorated<T: ?Sized, M: IMetadata>(_ptr: *const ItemHolder<T, M>) {
    #[cfg(feature = "validate")]
    {
        let raw = _ptr as usize & 3;
        if raw != 0 {
            panic!("Internal error in ArcShift - unexpected decorated pointer");
        }
    }
}

/// Return an undecorated version of the given pointer.
/// Supplying an already undecorated pointer is not an error, and returns
/// the value unmodified.
fn undecorate<T: ?Sized>(cand: *const ItemHolderDummy<T>) -> *const ItemHolderDummy<T> {
    let raw = cand as usize & 3;
    if raw != 0 { //TODO: Do we need this 'if'?
        ((cand as *const u8).wrapping_offset(-(raw as isize))) as *const ItemHolderDummy<T>
    } else {
        cand
    }
}

impl<T:?Sized> Clone for ArcShift<T> {
    fn clone(&self) -> Self {
        let t = with_holder!(self.item, T, |item: *const ItemHolder<T,_>| {
            do_clone_strong(item)
        });
        ArcShift {
            item: unsafe { NonNull::new_unchecked(t as *mut _) }
        }
    }
}
impl<T:?Sized> Clone for ArcShiftWeak<T> {
    fn clone(&self) -> Self {
        let t = with_holder!(self.item, T, |item: *const ItemHolder<T,_>| {
            do_clone_weak(item)
        });
        ArcShiftWeak {
            item: unsafe { NonNull::new_unchecked(t as *mut _) }
        }
    }
}

fn do_clone_strong<T:?Sized, M: IMetadata>(item_ptr: *const ItemHolder<T, M>) -> *const ItemHolderDummy<T> {
    let item = unsafe {&*item_ptr};
    debug_println!("Strong clone, about to access strong count of {:x?}", item_ptr);
    let strong_count = item.strong_count.fetch_add(1, Ordering::SeqCst);
    debug_println!("strong-clone, new strong count: {:x?} is {}", item_ptr, strong_count+1);
    debug_assert!(strong_count > 0);

    do_advance_strong::<T,M>(to_dummy::<T,M>(item_ptr))
}

fn do_clone_weak<T:?Sized, M: IMetadata>(item_ptr: *const ItemHolder<T, M>) -> *const ItemHolderDummy<T> {
    let item = unsafe {&*item_ptr};
    let weak_count = item.weak_count.fetch_add(1, Ordering::SeqCst);
    debug_println!("weak-clone, fetch_add, new weak count: {:x?} is {}", item_ptr, weak_count+1);
    debug_assert!(weak_count > 0);
    to_dummy(item_ptr)
}

fn do_upgrade_weak<T:?Sized, M: IMetadata>(mut item_ptr: *const ItemHolder<T, M>) -> Option<*const ItemHolderDummy<T>> {
    debug_println!("executing do_upgrade_weak");
    {
        let item = unsafe {&*(item_ptr)};
        //TODO: We could consume the original ref instead, if the 'upgrade' method took 'self' instead of '&self'
        item.weak_count.fetch_add(1, Ordering::SeqCst);
    }
    let mut item_ptr = to_dummy(item_ptr);
    loop {
        item_ptr = do_advance_weak::<T,M>(item_ptr);

        let item = unsafe {&*from_dummy::<T,M>(item_ptr)};
        let strong_count = item.strong_count.load(Ordering::SeqCst);
        if strong_count > 0 {
            if item.strong_count.compare_exchange(strong_count, strong_count + 1,Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                let reduce_weak = item.weak_count.fetch_sub(1, Ordering::SeqCst);
                debug_println!("upgrade success, new strong_count: {}, new weak: {}", strong_count + 1, reduce_weak - 1);
                return Some(item_ptr);
            }
            debug_println!("Race on strong_count _increase_ - loop.");
            continue; //Race on strong count, try again
        }

        //TODO: We should be able to upgrade even if strong count is 0, as long as the payload is not deallocated!
        if undecorate(item.next.load(Ordering::SeqCst)).is_null() {
            // next is still none, and the most recent item had a 0 strong count. We can't upgrade
            // to strong.
            debug_println!("Upgrade to strong failed");
            return None;
        }
        // Right most strong count was 0, but there's a next-pointer, so let's advance

    }
}




/// Returns an up-to-date pointer.
/// The closure is called with references to the old and new items, and allows
/// the caller to, for instance, decrease/inrcease strong counts. If no advance could be done,
/// this is always because we're already at the most up-to-date value. The closure is not called
/// in this case.
/// If the upgrade closure fails, this can only be because there's a new 'next', and the
/// current item has been dropped.
///
/// This function requires the caller to have some sort of reference to 'item_ptr', so that
/// it can't go away during execution. This method does not touch that reference count.
///
/// The 'upgrade' closure is called with a weak-ref taken on 'b', and 'a' in its original state.
/// It must either keep the weak ref, or possibly convert it to a strong ref.
/// It must also release whatever refcount it had on the original 'item_ptr'.
fn do_advance_impl<T: ?Sized, M: IMetadata>(mut item_ptr: *const ItemHolderDummy<T>, mut upgrade: impl FnMut(*const ItemHolder<T,M>,*const ItemHolder<T,M>) -> bool) -> *const ItemHolderDummy<T> {
    let start_ptr = item_ptr;

    debug_println!("advancing from {:x?}", item_ptr);
    loop {
        debug_println!("In advance-loop, item_ptr: {:x?}", item_ptr);
        let item: &ItemHolder<T,M> = unsafe { &*from_dummy(item_ptr  as *mut _) };
        //TODO: Use Relaxed load here, for a fast-path!
        let next_ptr = undecorate(item.next.load(Ordering::SeqCst));
        debug_println!("advancing from {:x?}, next_ptr = {:x?}", item_ptr, next_ptr);
        if next_ptr.is_null() {
            if item_ptr != start_ptr {
                if !upgrade(from_dummy(start_ptr), from_dummy(item_ptr)) {
                    debug_println!("upgrade failed, probably no payload");
                    continue;
                }
                //let start: &ItemHolder<T,M> = unsafe { &*from_dummy(start_ptr  as *mut _) };
                //let res = start.weak_count.fetch_sub(1, Ordering::SeqCst);
                //debug_println!("do_advance_impl: decrement weak of {:?}, was {}", start_ptr, res);
            }
            return item_ptr;
        }
        atomic::fence(Ordering::SeqCst);
        let advanced = item.advance_count.fetch_add(1, Ordering::SeqCst);
        debug_println!("advance: Increasing {:x?}.advance_count to {}", item_ptr, advanced+1);
        atomic::fence(Ordering::SeqCst);

        let next_ptr = undecorate(item.next.load(Ordering::SeqCst));
        assert_ne!(next_ptr, item_ptr); //TODO: Make debug_assert?
        let next: &ItemHolder<T,M> = unsafe { &*from_dummy(next_ptr) };
        debug_println!("advance: Increasing next(={:x?}).weak_count", next_ptr);
        let res = next.weak_count.fetch_add(1, Ordering::SeqCst);
        atomic::fence(Ordering::SeqCst);
        debug_println!("do_advance_impl: increment weak of {:?}, was {}, now {}, now accessing: {:x?}", next_ptr, res, res +1, item_ptr);

        let advanced = item.advance_count.fetch_sub(1, Ordering::SeqCst);
        atomic::fence(Ordering::SeqCst);
        debug_println!("advance: Decreasing {:x?}.advance_count to {}", item_ptr, advanced-1);

        if item_ptr != start_ptr {
            debug_println!("do_advance_impl: decrease weak from {:x?}", item_ptr);
            let res = item.weak_count.fetch_sub(1, Ordering::SeqCst);
            debug_println!("do_advance_impl: decrease weak from {:x?} - decreased to {}", item_ptr, res - 1);
        }
        item_ptr = next_ptr;
    }
}

fn do_advance_weak<T: ?Sized, M: IMetadata>(item_ptr: *const ItemHolderDummy<T>) -> *const ItemHolderDummy<T> {
    do_advance_impl(item_ptr, |a:*const ItemHolder<T,M>,b: *const ItemHolder<T,M>|{
        let a_weak = unsafe{(*a).weak_count.fetch_sub(1, Ordering::SeqCst)};
        // We have a weak ref count on 'b' given to use by `do_advance_impl`, which we're fine with
        debug_println!("weak advance {:x?}, decremented weak count to {}", a, a_weak.wrapping_sub(1));
        true
    })
}


fn do_advance_strong<T: ?Sized, M: IMetadata>(item_ptr: *const ItemHolderDummy<T>) -> *const ItemHolderDummy<T> {
    do_advance_impl::<_, M>(item_ptr, |a, b|{
        let mut b_strong = unsafe  { (*b).strong_count.load(Ordering::SeqCst) };
        loop {
            debug_println!("do_advance_strong:upgrading {:x?} -> {:x?} (b-count = {})", a,b,b_strong);
            if unsafe{ !(*b).has_payload()} {
                debug_println!("do_advance_strong:b has no payload");
                return false;
            }

            match unsafe { (*b).strong_count.compare_exchange(b_strong, b_strong + 1, Ordering::SeqCst, Ordering::SeqCst) } {
                Ok(_) => {
                    debug_println!("b-strong increased {:x?}: {} -> {}", b, b_strong, b_strong+1);
                    if unsafe{ !(*b).has_payload()} {
                        // Race - even though _did_ have payload prior to us grabbing the strong count, it now doesn't.
                        // This can only happen if there's another node to the right, that _does_ have a payload.
                        debug_println!("do_advance_strong:rare race, payload was dropped while advancing: {:?}", b);
                        unsafe { (*b).strong_count.fetch_sub(1, Ordering::SeqCst) };
                        return false;
                    }
                    if b_strong != 0 { //if b_strong was 0, we must take a weak-count on b, so instead we don't do the following sub
                        // Remove the weak count that do_advance_impl has let us inherit
                        let b_weak_count = unsafe{(*b).weak_count.fetch_sub(1, Ordering::SeqCst)};
                        debug_println!("strong advance {:x?}, reducing free b-count to {:?}", b, b_weak_count-1);
                    }
                    let a_strong = unsafe { (*a).strong_count.fetch_sub(1, Ordering::SeqCst)};
                    debug_println!("a-strong {:x?} decreased {} -> {} (weak of a: {})", a, a_strong, a_strong-1, unsafe{(*a).weak_count.load(Ordering::SeqCst)});
                    if a_strong == 1  {
                        do_drop_payload_if_possible(a);
                        // Remove the implicit weak granted by the strong
                        let a_weak = unsafe { (*a).weak_count.fetch_sub(1, Ordering::SeqCst)};
                        debug_println!("do_advance_strong:maybe dropping payload of a {:x?} (because strong-count is now 0). Adjusted weak to {}", a, a_weak.wrapping_sub(1));
                    }
                    return true;
                }
                Err(err_value) => {
                    debug_println!("do_advance_strong: race on strong_count for {:x?}", b);
                    b_strong = err_value;
                }
            }
        }
    })
}

fn raw_deallocate_node<T:?Sized, M:IMetadata>(item_ptr: *const ItemHolder<T, M>) {

    raw_do_unconditional_drop_payload_if_not_dropped(item_ptr as *mut ItemHolder<T, M>);

    assert_eq!(unsafe{ (*item_ptr).strong_count.load(Ordering::SeqCst)},0);

    let layout = get_holder_layout(&unsafe { &*item_ptr }.payload);
    // SAFETY:
    // fullptr is a valid uniquely owned pointer that we must deallocate
    debug_println!("Calling dealloc {:x?}", item_ptr);
    unsafe { std::alloc::dealloc(item_ptr as *mut _, layout) }
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
    Indeterminate
}


fn do_janitor_task<T: ?Sized, M: IMetadata>(start_ptr: *const ItemHolder<T, M>) -> NodeStrongStatus{

    let mut have_seen_left_strong_refs = false;
    let mut have_visited_leftmost = false;
    fn get_strong_status(strong:bool, leftmost: bool) -> NodeStrongStatus {
        if strong {
            NodeStrongStatus::StrongRefsFound
        } else if leftmost {
            NodeStrongStatus::NoStrongRefsExist
        } else {
            NodeStrongStatus::Indeterminate
        }
    }
    //TODO: Fast-path?
    // if start.prev.load(Ordering::Relaxed).is_null() { return ; }


    debug_println!("Janitor task for {:x?}", start_ptr);
    let start = unsafe { &*start_ptr };


    let start_ptr = to_dummy(start_ptr);
    if !start.lock_node_for_gc() {
        debug_println!("Janitor task for {:x?} - gc already active!", start_ptr);
        return get_strong_status(have_seen_left_strong_refs, have_visited_leftmost);
    }

    atomic::fence(Ordering::SeqCst);

    debug_println!("Loading prev of {:x?}", start_ptr);
    let mut cur_ptr : *const _ = start.prev.load(Ordering::SeqCst);
    debug_println!("Loaded prev of {:x?}, got: {:x?}", start_ptr, cur_ptr);
    let rightmost_candidate_ptr = cur_ptr;
    if cur_ptr.is_null() {
        debug_println!("Janitor task for {:x?} - no earlier nodes exist", start_ptr);
        // There are no earlier nodes to clean up
        start.unlock_node();
        return get_strong_status(have_seen_left_strong_refs, have_visited_leftmost);
    }

    let mut rightmost_deletable = null();

    loop {
        debug_println!("Accessing {:x?} in first janitor loop", cur_ptr);
        let cur: &ItemHolder<T,M> = unsafe { &*from_dummy(cur_ptr) };
        debug_println!("Setting next of {:x?} to {:x?}", cur_ptr, start_ptr);
        debug_assert_ne!(cur_ptr, start_ptr);
        cur.set_next(start_ptr);
        atomic::fence(Ordering::SeqCst);

        if !cur.lock_node_for_gc() {
            debug_println!("Janitor task for {:x?} - found node already being gced: {:x?}", start_ptr, cur_ptr);
            break;
        }

        if cur.strong_count.load(Ordering::SeqCst) > 0 {
            have_seen_left_strong_refs = true;
        }

        let prev_ptr: *const _ = cur.prev.load(Ordering::SeqCst);
        debug_println!("Janitor loop considering {:x?} -> {:x?}", cur_ptr, prev_ptr);
        if prev_ptr.is_null() {
            debug_println!("Janitor task for {:x?} - found leftmost node: {:x?}", start_ptr, cur_ptr);
            cur_ptr = null_mut();
            have_visited_leftmost = true;
            break;
        }

        let prev: &ItemHolder<T,M> = unsafe { &*from_dummy(prev_ptr) };
        debug_assert_ne!(prev_ptr, start_ptr);
        prev.set_next(start_ptr);
        atomic::fence(Ordering::SeqCst);
        let adv_count = prev.advance_count.load(Ordering::SeqCst);
        debug_println!("advance_count of {:x?} is {}", prev_ptr, adv_count);
        if adv_count > 0 {
            debug_println!("Janitor task for {:x?} - node {:x?} has advance in progress", start_ptr, prev_ptr);
            // All to the right of cur_ptr could be targets of the advance.
            // Because of races, we don't know that their target is actually 'start_ptr'
            // (though it could be).
            rightmost_deletable = cur_ptr;
        }
        cur_ptr = prev_ptr;
    }
    // The leftmost stop to this janitor cycle. This node must not be operated on.
    // I.e, it's one-beyond-the-end.
    let end_ptr = cur_ptr;

    fn find_non_deleted_predecessor<T:?Sized, M:IMetadata>(mut item_ptr: *const ItemHolderDummy<T>, last_valid: *const ItemHolderDummy<T>) -> Option<*const ItemHolderDummy<T>> {
        let mut deleted_count = 0;
        loop {
            if item_ptr == last_valid || item_ptr.is_null() {
                debug_println!("Find non-deleted {:x?}, count = {}, item_ptr = {:x?}, last_valid = {:x?}", item_ptr, deleted_count, item_ptr, last_valid);
                return (deleted_count > 0).then(|| item_ptr);
            }
            let item: &ItemHolder<T,M> = unsafe { &*from_dummy(item_ptr as *mut _) };
            // Item is *known* to have a 'next'!=null here, so
            // we know weak_count == 1 implies it is only referenced by its 'next'
            let item_weak_count = item.weak_count.load(Ordering::SeqCst);
            if item_weak_count > 1 {
                debug_println!("Find non-deleted {:x?}, count = {}, found node with weak count > 1 ({})", item_ptr, deleted_count, item_weak_count);
                return (deleted_count > 0).then(|| item_ptr);
            }
            debug_println!("Deallocating node in janitor task: {:x?}, dec: {:?}, weak count: {}, strong count: {}", item_ptr, unsafe{ (*from_dummy::<T,M>(item_ptr)).decoration()}, item_weak_count,
                item.strong_count.load(Ordering::SeqCst)
            );
            let prev_item_ptr = item.prev.load(Ordering::SeqCst);
            raw_deallocate_node(from_dummy::<T,M>(item_ptr as *mut _));
            deleted_count += 1;
            item_ptr = prev_item_ptr;
        }
    }


    let mut cur_ptr = rightmost_candidate_ptr;
    let mut right_ptr = start_ptr;
    let mut must_see_before_deletes_can_be_made = rightmost_deletable;
    debug_println!("Starting final janitor loop");
    while cur_ptr != end_ptr && cur_ptr != null_mut() {


        let new_predecessor =
            must_see_before_deletes_can_be_made
                .is_null()
                .then(||find_non_deleted_predecessor::<T, M>(cur_ptr, end_ptr)).flatten();

        if cur_ptr == must_see_before_deletes_can_be_made {
            debug_println!("Found must_see node: {:x?}", cur_ptr);
            // cur_ptr can't be deleted itself, but nodes further to the left can
            must_see_before_deletes_can_be_made = null_mut();
        }

        if let Some(new_predecessor_ptr) = new_predecessor {
            let right: &ItemHolder<T,M> = unsafe { &*from_dummy(right_ptr) };
            debug_println!("After janitor delete, setting {:x?}.prev = {:x?}", right_ptr, new_predecessor_ptr);
            right.prev.store(new_predecessor_ptr as *mut _, Ordering::SeqCst);
            atomic::fence(Ordering::SeqCst);

            if new_predecessor_ptr.is_null() {
                debug_println!("new_predecessor is null");
                break;
            }
            debug_println!("found new_predecessor: {:x?}", new_predecessor_ptr);
            right_ptr = new_predecessor_ptr;
            let new_predecessor = unsafe {(&*from_dummy::<T,M>(new_predecessor_ptr))};
            cur_ptr = new_predecessor.prev.load(Ordering::SeqCst);
            new_predecessor.unlock_node();
            debug_println!("New candidate advanced to {:x?}", cur_ptr);
        }
        else
        {
            debug_println!("Advancing to left, no node to delete found {:x?}", cur_ptr);
            let cur:&ItemHolder<T,M> = unsafe { &*from_dummy(cur_ptr) };
            right_ptr = cur_ptr;
            cur_ptr = unsafe {cur.prev.load(Ordering::SeqCst) };
            debug_println!("Advancing to left, advanced to {:x?}", cur_ptr);
            cur.unlock_node();
        }
    }
    debug_println!("Unlock start-node {:x?}", start_ptr);
    start.unlock_node();
    debug_println!("start-node unlocked: {:x?}", start_ptr);
    get_strong_status(have_seen_left_strong_refs, have_visited_leftmost)
}


/// drop_payload should be set to drop the payload of 'original_item_ptr' iff it is
/// either not rightmost, or all refs to all nodes are just weak refs (i.e, no strong refs exist)
fn do_drop_weak<T:?Sized, M:IMetadata>(mut original_item_ptr: *const ItemHolderDummy<T>) {
    debug_println!("drop weak {:x?}", original_item_ptr);
    let mut item_ptr = original_item_ptr;
    loop {
        debug_println!("drop weak loop {:?}", item_ptr);
        item_ptr = do_advance_weak::<T,M>(item_ptr);
        let strong_refs = do_janitor_task(from_dummy::<T,M>(item_ptr));

        // When we get here, we know we are advanced to the most recent node

        debug_println!("Accessing {:?}", item_ptr);
        let item: &ItemHolder<T,M> = unsafe { &*from_dummy(item_ptr) };
        let prior_weak = item.weak_count.load( Ordering::SeqCst);

        let next_ptr = item.next.load(Ordering::SeqCst);
        let have_next = !undecorate(next_ptr).is_null();
        let have_next_or_gc_active = !next_ptr.is_null();
        if !undecorate(next_ptr).is_null() {
            // drop_weak raced with 'add'.
            continue;
        }

        debug_println!("No add race");

        // We now have enough information to drop payload, if desired
        {
            let original_item = unsafe {&*from_dummy::<T,M>(item_ptr)};
            let original_strong = original_item.strong_count.load(Ordering::SeqCst);
            let original_next =  original_item.next.load(Ordering::SeqCst);
            if original_strong==0 && !get_decoration(original_next).is_dropped() {
                let mut can_drop_now;
                if !undecorate(original_next).is_null() {
                    can_drop_now = true;
                } else if strong_refs == NodeStrongStatus::NoStrongRefsExist {
                    can_drop_now = true;
                } else if have_next {
                    can_drop_now = true; //We have raced with an add, so there's definitely a new strongly reffed node to advance to!
                } else {
                    can_drop_now = false;
                }
                do_drop_payload_if_possible(original_item);
            }
        }


        let have_prev = !item.prev.load(Ordering::SeqCst).is_null();


        debug_println!("do_drop_weak: reducing weak count of {:x?}, to {} -> {} (have next/gc: {}, have prev: {}) ", item_ptr, prior_weak, prior_weak.wrapping_sub(1), have_next_or_gc_active,have_prev);
        match item.weak_count.compare_exchange(prior_weak, prior_weak - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => {}
            Err(_) => {
                debug_println!("drop weak {:x?}, raced on count decrement", item_ptr);
                continue;
            }
        }

        debug_println!("Prior weak of {:x?} is {}", item_ptr, prior_weak);
        if prior_weak == 1 {
            if !have_next_or_gc_active && !have_prev {
                if item.prev.load(Ordering::SeqCst).is_null() {
                    debug_println!("drop weak {:x?}, prior count = 1, raw deallocate", item_ptr);
                    raw_deallocate_node(from_dummy::<T,M>(item_ptr));
                } else {
                    debug_println!("drop weak {:x?}, couldn't drop node, because there are nodes to the left", item_ptr);
                }
            } else {
                debug_println!("{:x?} not doing final drop(weak), because have next, prev or gc active", item_ptr);
            }
        }
        return;
    }
}

fn do_update<T:?Sized, M:IMetadata>(initial_item_ptr: *const ItemHolder<T,M>, val_dummy: *const ItemHolderDummy<T>) -> *const ItemHolderDummy<T> {

    /*
    {
        let initial_item = unsafe {&*initial_item_ptr};
        if initial_item.strong_count.fetch_sub(1, Ordering::SeqCst) == 1 {
            // Reuse the weak-ref that the strong-ref implicitly held
            do_drop_payload(initial_item_ptr);
        } else {
            // We need to take a new weak ref, to represent the new node's ownership
            initial_item.weak_count.fetch_add(1, Ordering::SeqCst);
        }
    }
    */

    let mut item_ptr = to_dummy::<T,M>(initial_item_ptr);
    let new_node = val_dummy;

    debug_println!("Upgrading {:x?} to {:x?}", initial_item_ptr, val_dummy);
    let val = from_dummy::<T,M>(val_dummy) as *mut ItemHolder<T,M>;
    loop {
        item_ptr = do_advance_strong::<T,M>(item_ptr);
        let item = unsafe {&*from_dummy::<T,M>(item_ptr) };
        let cur_next = item.next.load(Ordering::SeqCst);
        if !undecorate(cur_next).is_null() {
            continue;
        }
        let new_next = decorate(new_node, get_decoration(cur_next));
        unsafe { (*val).prev = atomic::AtomicPtr::new(item_ptr as *mut _) };

        let weak_was = item.weak_count.fetch_add(1, Ordering::SeqCst);
        debug_assert!(weak_was > 0);
        debug_println!("do_update: increment weak of {:?} (was: {})", item_ptr, weak_was);

        atomic::fence(Ordering::SeqCst);
        match item.next.compare_exchange(cur_next, new_next as *mut _, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => {
                debug_println!("upgraded {:x?} to {:x?}", item_ptr, new_next);
            }
            Err(_) => {
                debug_println!("race, upgrade of {:x?} to {:x?} failed", item_ptr, new_next);
                let res = item.weak_count.fetch_sub(1, Ordering::SeqCst);
                debug_println!("race, decreasing {:x?} weak to {}", item_ptr, res-1);
                continue;
            }
        }


        let strong_count = item.strong_count.fetch_sub(1, Ordering::SeqCst);
        debug_println!("do_update: strong count {:x?} is now decremented to {} (weak = {})", item_ptr, strong_count-1, item.weak_count.load(Ordering::SeqCst));
        if strong_count == 1 {
            //TODO: We probably want to run the full janitor-task here...

            // It's safe to drop payload here, we've just now added a new item
            // that has its payload un-dropped, so there exists something to advance to.
            do_drop_payload_if_possible(from_dummy::<T,M>(item_ptr));
            let weak_count = item.weak_count.fetch_sub(1, Ordering::SeqCst);
            debug_println!("do_update: decrement weak of {:?}, new weak: {}", item_ptr, weak_count.saturating_sub(1));
        }
        return new_node;
    }
}


// Drops payload, unless either:
// * We're the rightmost node
// * The payload is already dropped
fn do_drop_payload_if_possible<T:?Sized, M:IMetadata>(item_ptr: *const ItemHolder<T,M>) {

    let item = unsafe {&*(item_ptr)};
    loop {
        let next_ptr = item.next.load(Ordering::SeqCst);
        let decoration = get_decoration(next_ptr);
        if decoration.is_dropped() || undecorate(next_ptr).is_null() {
            debug_println!("Not dropping {:x?} payload, because it's rightmost", item_ptr);
            return;
        }
        debug_println!("Drop if possible, {:x?}, cur dec: {:?}, dropped dec {:?}", item_ptr, decoration, decoration.dropped());
        match item.next.compare_exchange(next_ptr, decorate(next_ptr, decoration.dropped()) as *mut _, Ordering::SeqCst, Ordering::SeqCst) {
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
    debug_println!("Dropping payload {:x?} (dec: {:?})", item_ptr, unsafe { (*item_ptr).decoration()});
    let payload = unsafe { &(*item_ptr).payload };
    debug_println!("payload ref created");
    unsafe { ManuallyDrop::drop(unsafe { &mut *payload.get() }) };
    debug_println!("payload dropped");
}

fn raw_do_unconditional_drop_payload_if_not_dropped<T:?Sized, M:IMetadata>(item_ptr: *mut ItemHolder<T,M>) {
    debug_println!("Unconditional drop of payload {:x?}", item_ptr);
    let item = unsafe {&*(item_ptr)};
    let next_ptr = item.next.load(Ordering::SeqCst);
    let decoration = get_decoration(next_ptr);
    if !decoration.is_dropped() {
        debug_println!("Actual drop of payload {:x?}, it wasn't already dropped", item_ptr);
        let payload = unsafe { &(*item_ptr).payload };
        unsafe { ManuallyDrop::drop(unsafe { &mut *payload.get() }) };
        debug_println!("payload dropped");
    }
}

fn do_drop_strong<T: ?Sized, M: IMetadata>(full_item_ptr: *const ItemHolder<T,M>) {
    debug_println!("drop strong of {:x?} (strong count: {:?})", full_item_ptr, unsafe{(*full_item_ptr).strong_count.load(Ordering::SeqCst)});
    let mut item_ptr = to_dummy(full_item_ptr);

    item_ptr = do_advance_strong::<T,M>(item_ptr);
    let item:&ItemHolder<T,M> = unsafe { &*from_dummy(item_ptr) };

    #[cfg(test)]
    {
        let strong_count = item.strong_count.load(Ordering::SeqCst);
        assert!(strong_count > 0);
        let weak_count = item.weak_count.load(Ordering::SeqCst);
        assert!(weak_count > 0);
    }

    let prior_strong = item.strong_count.fetch_sub(1, Ordering::SeqCst);
    debug_println!("drop strong of {:x?}, prev count: {}, now {} (weak is {})", item_ptr, prior_strong, prior_strong-1,
        item.weak_count.load(Ordering::SeqCst));
    if prior_strong == 1 {
        // This is the only case where we actually might need to drop the rightmost node's payload
        // (if all nodes have only weak references)

        do_drop_weak::<T,M>(item_ptr);
    }

}

impl<T:?Sized> Drop for ArcShift<T> {
    fn drop(&mut self) {
        debug_println!("executing ArcShift::drop()");
        with_holder!(self.item, T, |item: *const ItemHolder<T,_>| {
            do_drop_strong(item)
        })

    }
}
impl<T:?Sized> Drop for ArcShiftWeak<T> {
    fn drop(&mut self) {

        fn drop_weak_helper<T:?Sized, M:IMetadata>(item: *const ItemHolder<T,M>) {
            do_drop_weak::<T,M>(to_dummy(item))
        }

        with_holder!(self.item, T, |item: *const ItemHolder<T,_>| {
            drop_weak_helper(item)
        })

    }
}

impl<T:?Sized> Deref for ArcShift<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        with_holder!(self.item, T, |item: *const ItemHolder<T,_>| -> &T {
            unsafe { (*item).payload() }
        })
    }
}

impl<T:?Sized> ArcShiftWeak<T> {
    pub fn upgrade(&self) -> Option<ArcShift<T>> {
        let t = with_holder!(self.item, T, |item: *const ItemHolder<T,_>| {
            do_upgrade_weak(item)
        });
        Some(ArcShift {
            item: unsafe { NonNull::new_unchecked(t? as *mut _) },
        })
    }

}
impl<T>  ArcShift<T> {
    pub fn new(val: T) -> ArcShift<T> {
        let holder = make_sized_holder(val, null_mut());
        atomic::fence(Ordering::SeqCst);
        ArcShift {
            item: unsafe { NonNull::new_unchecked(to_dummy(holder) as *mut _) },
        }
    }
    pub fn update(&mut self, val: T) {
        let holder = make_sized_holder(val, null_mut());

        with_holder!(self.item, T, |item: *const ItemHolder<T,_>| {
            let new_item = unsafe { NonNull::new_unchecked(do_update(item, std::mem::transmute_copy(&holder) ) as *mut _ ) };
            self.item = new_item;
        });
    }

}
impl<T:?Sized> ArcShift<T> {
    /// Basically the same as doing [`ArcShift::new`], but avoids copying the contents of 'input'
    /// to the stack, even as a temporary variable. This can be useful, if the type is too large
    /// to fit on the stack.
    pub fn from_box(input: Box<T>) -> ArcShift<T> {
        let holder = make_sized_or_unsized_holder_from_box(input, null_mut());
        ArcShift {
            // SAFETY:
            // from_box_impl never creates a null-pointer from a Box.
            item: unsafe { NonNull::new_unchecked(holder as *mut _) },
        }
    }

    pub fn update_box(&mut self, new_payload: Box<T>) {
        let holder = make_sized_or_unsized_holder_from_box(new_payload, null_mut());

        with_holder!(self.item, T, |item: *const ItemHolder<T,_>| {
            let new_item = unsafe { NonNull::new_unchecked(do_update(item, holder) as *mut _) };
            self.item = new_item;
        });
    }


    pub fn get(&mut self) -> &T {
        self.reload();
        &*self
    }

    // WARNING! This does not reload the pointer. You will see stale values.
    pub fn shared_get(&self) -> &T {
        &*self
    }

    #[must_use = "this returns a new `ArcShiftWeak` pointer, \
                  without modifying the original `ArcShift`"]
    pub fn downgrade(this: &ArcShift<T>) -> ArcShiftWeak<T> {
        let t = with_holder!(this.item, T, |item: *const ItemHolder<T,_>| {
            do_clone_weak(item)
        });
        ArcShiftWeak {
            item: unsafe { NonNull::new_unchecked(t as *mut _) }
        }
    }


    pub(crate) fn weak_count(&self) -> usize {
        with_holder!(self.item, T, |item: *const ItemHolder<T,_>| -> usize {
            unsafe { (&*item).weak_count.load(Ordering::SeqCst) }
        })
    }
    pub(crate) fn strong_count(&self) -> usize {
        with_holder!(self.item, T, |item: *const ItemHolder<T,_>| -> usize {
            unsafe { (&*item).strong_count.load(Ordering::SeqCst) }
        })
    }
    pub fn reload(&mut self) {
        fn advance_strong_helper<T:?Sized,M:IMetadata>(ptr: *const ItemHolder<T,M>) -> *const ItemHolderDummy<T> {
            do_advance_strong::<T,M>(to_dummy(ptr))
        }
        let advanced = with_holder!(self.item, T, |item: *const ItemHolder<T,_>| {
            advance_strong_helper(item)
        });
        self.item = unsafe { NonNull::new_unchecked(advanced as *mut _ ) };
    }

    /// Check all links and refcounts.
    ///
    /// # SAFETY
    /// This method requires that no other threads access the chain while it is running.
    /// It is up to the caller to actually ensure this.
    pub(crate) unsafe fn debug_validate(strong_handles: &[&Self], weak_handles: &[&ArcShiftWeak<T>]) {
        let first = &strong_handles[0];
        with_holder!(first.item, T, |item: *const ItemHolder<T,_>| {
            Self::debug_validate_impl(strong_handles, weak_handles, item);
        });
    }
    fn debug_validate_impl<M:IMetadata>(
        strong_handles: &[&ArcShift<T>],
        weak_handles: &[&ArcShiftWeak<T>],
        _prototype: *const ItemHolder<T,M>) {
        let mut last = from_dummy(strong_handles[0].item.as_ptr());

        debug_println!("Start traverse right at {:?}", last);
        loop {
            debug_println!("loop traverse right at {:?}", last);
            let next = unsafe{undecorate((*last).next.load(Ordering::SeqCst))};
            debug_println!("Traversed to {:?}", next);
            if next.is_null() { break;}
            last = from_dummy::<T,M>(next);
        }
        {
            debug_println!("Reading {:?}.next", to_dummy(last));
            let last_next = unsafe{(*last).next.load(Ordering::SeqCst)};
            debug_println!("Reading {:?}.next gave {:?}", to_dummy(last), last_next);
            assert!(!get_decoration(last_next).is_dropped(), "the rightmost node must never be dropped (at least at rest, but regardless of refcount)");
        }

        let mut true_weak_refs = HashMap::<*const ItemHolderDummy<T>,usize>::new();
        let mut true_strong_refs = HashMap::<*const ItemHolderDummy<T>,usize>::new();
        let all_nodes = unsafe {(*last).debug_all_to_left()};
        for node in all_nodes.iter() {
            debug_println!("Node: {:?}", *node as *const ItemHolder<T,_>);
        }
        for node in all_nodes.iter().copied() {
            let next = unsafe{(*last).next.load(Ordering::SeqCst)};
            if !next.is_null() {
                assert!(all_nodes.contains(unsafe {&&*from_dummy::<T,M>(next)}));
            }
            let prev: *const _ = node.prev.load(Ordering::SeqCst);
            if !prev.is_null() {
                *true_weak_refs.entry(prev).or_default() += 1;
                assert!(all_nodes.contains(unsafe {&&*from_dummy::<T,M>(prev)}));
            }
            let advance_count = unsafe{(*last).advance_count.load(Ordering::SeqCst)};
            assert_eq!(advance_count, 0, "advance_count must always be 0 at rest");
        }

        for handle in weak_handles.iter() {
            assert!(all_nodes.contains(unsafe {&&*from_dummy::<T,M>(handle.item.as_ptr())}));
            let true_weak_count = true_weak_refs.entry(handle.item.as_ptr()).or_default();
            *true_weak_count += 1;
        }
        for handle in strong_handles.iter() {
            assert!(all_nodes.contains(unsafe {&&*from_dummy::<T,M>(handle.item.as_ptr())}));
            let handle_next: *const _ = unsafe{(*from_dummy::<T,M>(handle.item.as_ptr())).next.load(Ordering::SeqCst)};
            assert!(!get_decoration(handle_next).is_dropped(), "nodes referenced by strong handles must not be dropped, but {:?} was", handle.item.as_ptr());

            let true_strong_count = true_strong_refs.entry(handle.item.as_ptr()).or_default();
            if *true_strong_count == 0 {
                *true_weak_refs.entry(handle.item.as_ptr()).or_default() += 1;
            }
            *true_strong_count += 1;
        }
        for node in all_nodes.iter().copied() {
            let node_dummy = to_dummy(node);
            let strong_count = node.strong_count.load(Ordering::SeqCst);
            let weak_count = node.weak_count.load(Ordering::SeqCst);
            let expected_strong_count = true_strong_refs.get(&node_dummy).unwrap_or(&0);
            let expected_weak_count = true_weak_refs.get(&node_dummy).unwrap_or(&0);
            assert_eq!(strong_count, *expected_strong_count, "strong count of {:x?} should be {}", node as *const _, *expected_strong_count);
            assert_eq!(weak_count, *expected_weak_count, "weak count of {:x?} should be {}", node as *const _, *expected_weak_count);

            let next = node.next.load(Ordering::SeqCst);
            /*if strong_count == 0 && !undecorate(next).is_null() {
                assert!(get_decoration(next).is_dropped(), "if strong count is 0, node should be dropped"); //<- Perhaps this isn't actually a hard guarantee, if this fails, consider if it's expected?
            }*/
            if strong_count >= 0 {
                assert!(!get_decoration(next).is_dropped(), "if strong count is >0, node should not be dropped");
            }
        }
    }
}


pub mod new_tests {
    use std::thread;
    use crate::ArcShift;

    #[test]
    fn simple_create() {
        let mut x = ArcShift::new(Box::new(45u32));
        assert_eq!(**x, 45);
        unsafe { ArcShift::debug_validate(&[&x],&[]) };
    }
    #[test]
    fn simple_create_and_update() {
        let mut x = ArcShift::new(Box::new(45u32));
        assert_eq!(**x, 45);
        x.update(Box::new(1u32));
        assert_eq!(**x, 1);
        unsafe { ArcShift::debug_validate(&[&x],&[]) };
    }
    #[test]
    fn simple_create_and_update_twice() {
        let mut x = ArcShift::new(Box::new(45u32));
        assert_eq!(**x, 45);
        x.update(Box::new(1u32));
        assert_eq!(**x, 1);
        x.update(Box::new(21));
        assert_eq!(**x, 21);
        unsafe { ArcShift::debug_validate(&[&x],&[]) };
    }
    #[test]
    fn simple_create_and_clone() {
        let mut x = ArcShift::new(Box::new(45u32));
        let y = x.clone();
        assert_eq!(**x, 45);
        assert_eq!(**y, 45);
        unsafe { ArcShift::debug_validate(&[&x,&y],&[]) };
    }
    #[test]
    fn simple_create_and_clone_drop_other_order() {
        let mut x = ArcShift::new(Box::new(45u32));
        let y = x.clone();
        assert_eq!(**x, 45);
        assert_eq!(**y, 45);
        unsafe { ArcShift::debug_validate(&[&x,&y],&[]) };
        drop(x);
        drop(y);
    }
    #[test]
    fn simple_downgrade() {
        let mut x = ArcShift::new(Box::new(45u32));
        let y = ArcShift::downgrade(&x);
    }
    #[test]
    fn simple_create_and_clone_and_update1() {
        let mut right = ArcShift::new(Box::new(1u32));
        assert_eq!(right.strong_count(), 1);
        assert_eq!(right.weak_count(), 1);
        let left = right.clone();
        right.update(Box::new(2u32)); // becomes right here
        assert_eq!(right.strong_count(), 1);
        assert_eq!(right.weak_count(), 1);
        assert_eq!(**right, 2);
        assert_eq!(**left, 1);
        assert_eq!(left.strong_count(), 1);
        assert_eq!(left.weak_count(), 2); //'left' and ref from right
        debug_println!("Dropping 'left'");
        unsafe { ArcShift::debug_validate(&[&left,&right],&[]) };
        drop(left);
        assert_eq!(right.strong_count(), 1);
        assert_eq!(right.weak_count(), 1);

    }
    #[test]
    fn simple_create_and_clone_and_update_other_drop_order() {
        let mut x = ArcShift::new(Box::new(1u32));
        assert_eq!(x.strong_count(), 1);
        assert_eq!(x.weak_count(), 1);
        let y = x.clone();
        assert_eq!(y.strong_count(), 2);
        assert_eq!(y.weak_count(), 1);
        x.update(Box::new(2u32));
        assert_eq!(x.strong_count(), 1);
        assert_eq!(x.weak_count(), 1);

        assert_eq!(y.strong_count(), 1);
        assert_eq!(y.weak_count(), 2);

        assert_eq!(**x, 2);
        assert_eq!(**y, 1);

        unsafe { ArcShift::debug_validate(&[&x,&y],&[]) };
        debug_println!("Dropping x");
        drop(x);
        debug_println!("Dropping y");
        drop(y);
    }

    #[test]
    fn simple_create_clone_and_update_twice() {
        let mut x = ArcShift::new(Box::new(45u32));
        let mut y = x.clone();
        assert_eq!(**x, 45);
        x.update(Box::new(1u32));
        assert_eq!(**x, 1);
        x.update(Box::new(21));
        assert_eq!(**x, 21);
        unsafe { ArcShift::debug_validate(&[&x,&y],&[]) };
        y.reload();
        assert_eq!(**y, 21);
        unsafe { ArcShift::debug_validate(&[&x,&y],&[]) };
    }

    #[test]
    fn simple_create_clone_twice_and_update_twice() {
        let mut x = ArcShift::new(Box::new(45u32));
        let mut y = x.clone();
        unsafe { ArcShift::debug_validate(&[&x,&y],&[]) };
        assert_eq!(**x, 45);
        x.update(Box::new(1u32));
        unsafe { ArcShift::debug_validate(&[&x,&y],&[]) };
        let z = x.clone();
        assert_eq!(**x, 1);
        x.update(Box::new(21));
        unsafe { ArcShift::debug_validate(&[&x,&y,&z],&[]) };
        assert_eq!(**x, 21);
        y.reload();
        assert_eq!(**y, 21);
        unsafe { ArcShift::debug_validate(&[&x,&y,&z],&[]) };
    }

    #[test]
    fn simple_threaded() {
        let mut arc = ArcShift::new("Hello".to_string());
        let mut arc2 = arc.clone();

        let j1 =
            thread::Builder::new().name("thread1".to_string()).spawn(move || {
            //println!("Value in thread 1: '{}'", *arc); //Prints 'Hello'
            arc.update("New value".to_string());
            //println!("Updated value in thread 1: '{}'", *arc); //Prints 'New value'
        }).unwrap();

        let j2 = thread::Builder::new().name("thread2".to_string()).spawn(move || {
            // Prints either 'Hello' or 'New value', depending on scheduling:
            let _a = arc2;
            //println!("Value in thread 2: '{}'", arc2.get());
        }).unwrap();

        j1.join().unwrap();
        j2.join().unwrap();

    }

    #[test]
    fn simple_upgrade_reg() {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let shift = ArcShift::new(crate::tests::leak_detection::InstanceSpy::new(count.clone()));
        unsafe { ArcShift::debug_validate(&[&shift], &[]) };
        let shiftlight = ArcShift::downgrade(&shift);
        unsafe { ArcShift::debug_validate(&[&shift], &[&shiftlight]) };

        debug_println!("==== running shift.get() = ");
        let mut shift2 = shiftlight.upgrade().unwrap();
        debug_println!("==== running arc.update() = ");
        unsafe { ArcShift::debug_validate(&[&shift,&shift2], &[&shiftlight]) };
        shift2.update(crate::tests::leak_detection::InstanceSpy::new(count.clone()));

        unsafe { ArcShift::debug_validate(&[&shift,&shift2], &[&shiftlight]) };
    }

}

#[cfg(test)]
mod tests2 {
    use std::sync::Arc;
    use crate::{atomic, ArcShift};

    #[cfg(all(not(loom), not(feature = "shuttle")))]
    fn model(x: impl FnOnce()) {
        x()
    }
    #[cfg(loom)]
    fn model(x: impl Fn() + 'static + Send + Sync) {
        loom::model(x)
    }

    #[cfg(all(feature = "shuttle", coverage))]
    const SHUTTLE_ITERATIONS: usize = 50;
    #[cfg(all(feature = "shuttle", not(coverage)))]
    const SHUTTLE_ITERATIONS: usize = 50;

    #[cfg(feature = "shuttle")]
    fn model(x: impl Fn() + 'static + Send + Sync) {
        shuttle::check_random(x, SHUTTLE_ITERATIONS);
    }

    #[test]
    fn simple_test_clones2() {
        model(|| {
            let shift = ArcShift::new("orig".to_string());
            let shift1 = ArcShift::downgrade(&shift);
            let shift2 = shift.clone();
            let shift3 = shift.clone();
            unsafe { ArcShift::debug_validate(&[&shift,&shift2,&shift3],&[&shift1]) };
        });
    }
    #[test]
    fn simple_test_clonesp2() {
        let shift = ArcShift::new("orig".to_string());
    }
    #[test]
    fn simple_threading_update_in_one() {
        model(|| {
            debug_println!("-------- loom -------------");
            let mut shift = ArcShift::new(42u32);
            let mut shift1 = shift.clone();
            let t1 = atomic::thread::Builder::new()
                .name("t1".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    shift.update(43);
                    debug_println!("t1 dropping");
                })
                .unwrap();

            let t2 = atomic::thread::Builder::new()
                .name("t2".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    std::hint::black_box(shift1.get());
                    debug_println!("t2 dropping");
                })
                .unwrap();
            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
        });
    }

    #[test]
    fn simple_threading_update_twice() {
        model(|| {
            debug_println!("-------- loom -------------");
            let mut shift = ArcShift::new(42u32);
            let mut shift1 = shift.clone();
            let mut shift2 = shift.clone();
            unsafe { ArcShift::debug_validate(&[&shift, &shift1, &shift2],&[]) };
            let t1 = atomic::thread::Builder::new()
                .name("t1".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    shift.update(43);
                    debug_println!("--> t1 dropping");
                })
                .unwrap();

            let t2 = atomic::thread::Builder::new()
                .name("t2".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    shift1.update(44);
                    debug_println!("--> t2 dropping");
                })
                .unwrap();
            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            unsafe { ArcShift::debug_validate(&[&shift2],&[]) };
            println!("--> Main dropping");
            assert!(*shift2.get() > 42);
        });
    }
    #[test]
    fn simple_threading_update_thrice() {
        model(|| {
            debug_println!("-------- loom -------------");
            let mut shift = ArcShift::new(42u32);
            let mut shift1 = shift.clone();
            let mut shift2 = shift.clone();
            let mut shift3 = shift.clone();
            unsafe { ArcShift::debug_validate(&[&shift, &shift1, &shift2,&shift3],&[]) };
            let t1 = atomic::thread::Builder::new()
                .name("t1".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    shift.update(43);
                    debug_println!("--> t1 dropping");
                })
                .unwrap();

            let t2 = atomic::thread::Builder::new()
                .name("t2".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    shift1.update(44);
                    debug_println!("--> t2 dropping");
                })
                .unwrap();

            let t3 = atomic::thread::Builder::new()
                .name("t3".to_string())
                .stack_size(1_000_000)
                .spawn(move || {
                    shift2.update(45);
                    debug_println!("--> t3 dropping");
                })
                .unwrap();
            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            _ = t3.join().unwrap();
            unsafe { ArcShift::debug_validate(&[&shift3],&[]) };
            println!("--> Main dropping");
            assert!(*shift3.get() > 42);
        });
    }

}


// Module for tests
//TODO: Don't comment this out!
#[cfg(test)]
pub mod tests;
