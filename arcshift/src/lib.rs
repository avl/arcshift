#![cfg_attr(feature = "nightly", feature(ptr_metadata))]
#![deny(warnings)]
#![forbid(clippy::undocumented_unsafe_blocks)]
#![deny(missing_docs)]

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
//! [`ArcShiftLight::reload`], that instance advances along the linked list to the last
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
//!   This can be partially mitigated by using the [`ArcShiftLight`]-type for long-lived
//!   never-reloaded instances.
//! * Modifying the value is approximately 10x more expensive than modifying `Arc<RwLock<T>>`
//! * When the value is modified, the next subsequent access can be slower than an `Arc<RwLock<T>>`
//!   access.
//! * ArcShift is its own datatype. It is in no way compatible with `Arc<T>`.
//! * At most 524287 instances of ArcShiftLight can be created for each value.
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
//! ArcShiftLight-instances also keep pointers to the heap blocks mentioned above, but value T
//! in the block can be dropped while being held by an ArcShiftLight. This means that ArcShiftLight-
//! instances only consume `std::mem::size_of::<T>()` bytes of memory, when the value they
//! point to has been dropped. When the ArcShiftLight-instances is reloaded, or dropped, that memory
//! is also released.
//!
//!
//! # Pitfall #1 - lingering memory usage
//!
//! Be aware that ArcShift instances that are just "lying around" without ever being reloaded,
//! will keep old values around, taking up memory. This is a fundamental drawback of the approach
//! taken by ArcShift. One workaround is to replace long-lived non-reloaded instances of
//! [`ArcShift`] with [`ArcShiftLight`]. This alleviates the problem.
//!
//! # Pitfall #2 - reference count limitations
//!
//! ArcShift uses a single 64 bit reference counter to track both ArcShift
//! and ArcShiftLight instance counts. This is achieved by giving each ArcShiftLight-instance a
//! weight of 1, while each ArcShift-instance receives a weight of 524288. As a consequence
//! of this, the maximum number of ArcShiftLight-instances (for the same value), is 524287.
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

use crate::ItemStateEnum::{Dropped, Superseded};
use std::alloc::Layout;
#[allow(unused)]
use std::backtrace::Backtrace;
use std::cell::{Cell, UnsafeCell};
use std::fmt::{Debug, Display, Formatter};
use std::marker::PhantomData;
use std::mem;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ops::Deref;
use std::panic::UnwindSafe;
use std::ptr::{addr_of_mut, drop_in_place, null, null_mut, NonNull};
use std::sync::atomic::Ordering;

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
    pub use std::hint::spin_loop;
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

/// The maximum number of roots that can be created for a
const MAX_ROOTS: usize = 524288;
const MAX_ARCSHIFT: usize = 35000000000000;

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
pub struct ArcShift<T: 'static + ?Sized> {
    item: NonNull<ItemHolderDummy<T>>,
}

impl<T> UnwindSafe for ArcShift<T> {}

/// ArcShiftLight is like ArcShift, except it does not provide overhead-free access.
/// However, it has the advantage of not preventing old versions of the payload type from being
/// freed.
///
/// ```rust
/// # #[cfg(not(any(loom,feature="shuttle")))]
/// # {
/// # extern crate arcshift;
/// # use arcshift::ArcShiftLight;
/// let light_instance = ArcShiftLight::new("test");
/// let instance = light_instance.upgrade();
/// println!("Value: {:?}", *instance);
/// # }
/// ```
///
/// WARNING! Because of implementation reasons, each instance of ArcShiftLight will claim
/// a memory equal to `size_of::<T>` (plus a bit), even if the value inside it has been moved out,
/// and even if all other instances of ArcShift/ArcShiftLight have been dropped.
/// If this limitation is unacceptable, consider using `ArcShiftLight<Box<T>>` as your datatype,
/// or possibly using a different crate.
pub struct ArcShiftLight<T: 'static + ?Sized> {
    item: NonNull<ItemHolderDummy<T>>,
}

/// SAFETY:
/// ArcShiftLight can be Send as long as T is Send.
/// ArcShiftLight's mechanisms are compatible with both Send and Sync
unsafe impl<T: 'static> Send for ArcShiftLight<T> where T: Send {}

/// SAFETY:
/// ArcShiftLight can be Sync as long as T is Sync.
/// ArcShiftLight's mechanisms are compatible with both Send and Sync
unsafe impl<T: 'static> Sync for ArcShiftLight<T> where T: Sync {}

/// ArcShiftCell is like an ArcShift, except that it can be reloaded
/// without requiring 'mut'-access.
/// However, it is not 'Sync'.
pub struct ArcShiftCell<T: 'static> {
    inner: UnsafeCell<ArcShift<T>>,
    recursion: Cell<usize>,
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

impl<T: 'static> Clone for ArcShiftCell<T> {
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
/// a 'get'-function closure.
pub struct RecursionDetected;

impl Debug for RecursionDetected {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "RecursionDetected")
    }
}

impl Display for RecursionDetected {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "RecursionDetected")
    }
}

impl std::error::Error for RecursionDetected {}

impl<T: 'static> ArcShiftCell<T> {
    /// Create a new ArcShiftCell with the given value.
    pub fn new(value: T) -> ArcShiftCell<T> {
        ArcShiftCell::from_arcshift(ArcShift::new(value))
    }
    /// Creates an ArcShiftCell from an ArcShift-instance.
    /// The payload is not cloned, the two pointers keep pointing to the same object.
    pub fn from_arcshift(input: ArcShift<T>) -> ArcShiftCell<T> {
        ArcShiftCell {
            inner: UnsafeCell::new(input),
            recursion: Cell::new(0),
        }
    }
    /// Get the value pointed to.
    ///
    /// This method is very fast, basically the speed of a regular reference, unless
    /// the value has been modified by calling one of the update-methods.
    ///
    /// This method will drop older values which are no longer needed
    /// This method is reentrant - you are allowed to call it from within the closure 'f'.
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
    /// the 'ArcShiftCell::get'-function.
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
    /// Note, this function only works when not called from within a closure
    /// supplied to the 'get' function. If such recursion occurs, this function
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

const fn is_sized<T: ?Sized>() -> bool {
    size_of::<&T>() == size_of::<&()>()
}

impl<T: 'static> Clone for ArcShiftLight<T> {
    fn clone(&self) -> Self {
        let mut curitem = self.item.as_ptr() as *const _;
        loop {
            let Some(next) = Self::load_nontentative_next(curitem) else {
                atomic::spin_loop();
                continue;
            };
            if !next.is_null() {
                curitem = undecorate(next);
                atomic::spin_loop();
                continue;
            }
            let count = get_refcount(curitem).load(Ordering::Relaxed);
            assert_ne!(count, 0);
            Self::verify_count(count);
            match get_refcount(curitem).compare_exchange(
                count,
                count + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    debug_println!(
                        "ArcShiftLight clone count successfully updated to: {}",
                        count + 1
                    );
                    break;
                }
                Err(othercount) => {
                    Self::verify_count(othercount);
                    debug_println!("ArcShiftLight clone count race {:?}", curitem);

                    atomic::spin_loop();
                    continue;
                }
            }
        }
        debug_println!("Returning new ArcShiftLight for {:?}", curitem);
        ArcShiftLight { item:
            // SAFETY:
            // Conditions above guarantee that curitem is not null
            unsafe { NonNull::new_unchecked(curitem as *mut _)
        } }
    }
}

fn make_sized_holder<T, M: IMetadata>(item: ItemHolder<T, M>) -> *mut ItemHolderDummy<T> {
    let cur_ptr = Box::into_raw(Box::new(item));
    cur_ptr as _
}

#[repr(transparent)]
struct Metadata<T: ?Sized> {
    #[cfg(feature = "nightly")]
    meta: <T as std::ptr::Pointee>::Metadata,
    #[cfg(not(feature = "nightly"))]
    meta: *const (),
    phantom: PhantomData<T>,
}
impl<T: ?Sized> Clone for Metadata<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: ?Sized> Copy for Metadata<T> {}
impl<T: ?Sized> std::fmt::Debug for Metadata<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Metadata({:?})", self.meta)
    }
}

#[allow(dead_code)]
#[repr(C)]
struct FatPtr<T: ?Sized> {
    ptr: *mut u8,
    meta: Metadata<T>,
}

#[inline]
fn arc_from_raw_parts_mut<T: ?Sized, M: IMetadata>(
    data_ptr: *mut u8,
    metadata: Metadata<T>,
) -> *mut ItemHolder<T, M> {
    // SAFETY:
    // This is the best I managed without using nightly-only features (August 2024).
    // It is safe as long as the actual internal representation of fat pointers doesn't change.
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
    metadata: Metadata<T>,
) -> *const ItemHolder<T, M> {
    // SAFETY:
    // This is the best I managed without using nightly-only features (August 2024).
    // It is safe as long as the actual internal representation of fat pointers doesn't change.
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

impl<T: ?Sized> Metadata<T> {
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
    pub fn new(cur_ptr: *const T) -> Metadata<T> {
        Metadata {
            #[cfg(feature = "nightly")]
            meta: std::ptr::metadata(cur_ptr),
            #[cfg(not(feature = "nightly"))]
            meta: Self::polyfill_metadata(cur_ptr),
            phantom: PhantomData,
        }
    }
}

fn get_holder_layout<T: ?Sized + 'static, M: IMetadata>(ptr: *const T) -> Layout {
    // SAFETY:
    // The pointer 'ptr' is a valid pointer
    let payload_layout = Layout::for_value(unsafe { &*ptr });
    if is_sized::<T>() {
        let layout = Layout::new::<ItemHolder<(), NoMeta>>();
        let (layout, _) = layout.extend(payload_layout).unwrap();
        layout.pad_to_align()
    } else {
        let layout = Layout::new::<Metadata<T>>();
        let (layout, _) = layout
            .extend(Layout::new::<ItemHolder<(), Metadata<T>>>())
            .unwrap();
        let (layout, _) = layout.extend(payload_layout).unwrap();
        layout.pad_to_align()
    }
}

#[allow(unused)]
fn make_unsized_holder_from_box<T: ?Sized>(item: Box<T>) -> *const ItemHolderDummy<T> {
    let cur_ptr = Box::into_raw(item);

    debug_println!("thesize: {:?}", cur_ptr);
    let item_holder_ptr: *mut ItemHolder<T, Metadata<T>>;

    // SAFETY:
    // The pointer 'cur_ptr' is a valid pointer (it's just been created by 'Box::into_raw'
    let payload_layout = Layout::for_value(unsafe { &*cur_ptr });
    let the_size = payload_layout.size();

    if is_sized::<T>() {
        unreachable!()
    } else {
        let layout = get_holder_layout::<T, Metadata<T>>(cur_ptr);

        let metadata = Metadata::new(cur_ptr);
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
            addr_of_mut!((*item_holder_ptr).the_size).write(metadata);
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
        addr_of_mut!((*item_holder_ptr).next_and_state).write(atomic::AtomicPtr::default());
    }
    // SAFETY:
    // refcount is just an AtomicUsize-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).refcount).write(atomic::AtomicUsize::new(MAX_ROOTS));
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
    // Converting it to a Box<MaybeUninit<T>> is safe, and will make sure that any drop-function
    // of T is not run. We must not drop T here, since we've moved it to the 'result_ptr'.
    let _t: Box<ManuallyDrop<T>> = unsafe { Box::from_raw(cur_ptr as *mut ManuallyDrop<T>) }; //Free the memory, but don't drop 'input'

    let ret = item_holder_ptr as *const ItemHolderDummy<T>;
    debug_println!(
        "ret item holder dummy ptr: {:?}, next: {:?}",
        ret,
        // SAFETY:
        // 'item_holder_ptr' is a valid pointer.
        unsafe { (*item_holder_ptr).next_and_state.load(Ordering::SeqCst) }
    );
    debug_println!("Next : {:?}", get_next_and_state(ret));
    ret
}

#[allow(unused)]
fn make_sized_holder_from_box<T: ?Sized>(item: Box<T>) -> *const ItemHolderDummy<T> {
    let cur_ptr = Box::into_raw(item);

    debug_println!("thesize: {:?}", cur_ptr);

    // SAFETY:
    // 'cur_ptr' is a valid pointer, it's just been created by Box::into_raw
    let payload_layout = Layout::for_value(unsafe { &*cur_ptr });
    let the_size = payload_layout.size();

    let layout = get_holder_layout::<T, NoMeta>(cur_ptr);

    // SAFETY:
    // The type '*mut ItemHolder<T, NoMeta>' is not actually a fat pointer. But since T:?Sized,
    // the compiler treats it like a fat pointer with a zero-size metadata, which is not
    // the exact same thing as a thin pointer (is my guess).
    // Using transmute_copy to trim off the metadata is sound.
    let item_holder_ptr: *mut ItemHolder<T, NoMeta> =
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
        addr_of_mut!((*item_holder_ptr).next_and_state).write(atomic::AtomicPtr::default());
    }
    // SAFETY:
    // refcount is just an AtomicUsize-type, for which all bit patterns are valid.
    unsafe {
        addr_of_mut!((*item_holder_ptr).refcount).write(atomic::AtomicUsize::new(MAX_ROOTS));
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
    // Converting it to a Box<MaybeUninit<T>> is safe, and will make sure that any drop-function
    // of T is not run. We must not drop T here, since we've moved it to the 'result_ptr'.
    let _t: Box<ManuallyDrop<T>> = unsafe { Box::from_raw(cur_ptr as *mut ManuallyDrop<T>) }; //Free the memory, but don't drop 'input'

    let ret = item_holder_ptr as *const ItemHolderDummy<T>;
    debug_println!(
        "ret item holder dummy ptr: {:?}, next: {:?}",
        ret,
        // SAFETY:
        // item_holder_ptr is a valid pointer
        unsafe { (*item_holder_ptr).next_and_state.load(Ordering::SeqCst) }
    );
    debug_println!("Next : {:?}", get_next_and_state(ret));
    ret
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
        let ptr_data = dummy as *const _;
        debug_println!("Dummy data: {:?}", ptr_data);
        let metadata_ptr = dummy as *const Metadata<T>;
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
        arc_from_raw_parts(ptr_data, metadata)
    }
}
impl<T: 'static + ?Sized> ArcShiftLight<T> {
    /// Load 'next'.
    /// If 'next' is tentative, convert it to superseded.
    /// 'curitem' must be a valid pointer.
    #[inline]
    fn load_nontentative_next(
        curitem: *const ItemHolderDummy<T>,
    ) -> Option<*const ItemHolderDummy<T>> {
        let next = get_next_and_state(curitem).load(Ordering::SeqCst);

        debug_println!(
            "load_nontentative_next upgrade {:?}, next: {:?} = {:?}",
            curitem,
            next,
            get_state(next)
        );
        if get_state(next) == Some(ItemStateEnum::SupersededByTentative) {
            match get_next_and_state(curitem).compare_exchange(
                next,
                decorate(next, ItemStateEnum::Superseded) as *mut _,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    debug_println!(
                        "ArcShiftLight {:?} tentative upgrade, returning {:?}",
                        curitem,
                        decorate(next, ItemStateEnum::Superseded)
                    );
                    Some(decorate(next, ItemStateEnum::Superseded))
                }
                Err(_) => {
                    debug_println!(
                        "Race while upgrading ArcShiftLight to ArcShift: {:?}",
                        curitem
                    );
                    None
                }
            }
        } else {
            debug_println!("load_nontentative_next {:?} returning {:?}", curitem, next);
            Some(next)
        }
    }
}

impl<T: 'static> ArcShiftLight<T> {
    /// Create a new ArcShiftLight-instance, containing the given value.
    pub fn new(payload: T) -> ArcShiftLight<T> {
        let item = ItemHolder {
            the_size: NoMeta,
            #[cfg(feature = "validate")]
            magic1: std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111),
            #[cfg(feature = "validate")]
            magic2: std::sync::atomic::AtomicU64::new(0x1234123412348111),
            payload,
            next_and_state: atomic::AtomicPtr::default(),
            refcount: atomic::AtomicUsize::new(1),
        };
        let cur_ptr = make_sized_holder(item);
        debug_println!("Created ArcShiftLight for {:?}", cur_ptr);
        ArcShiftLight { item:
            // SAFETY:
            // cur_ptr is not null, since it is the result of Box::into_raw
            unsafe {NonNull::new_unchecked(cur_ptr)
        } }
    }

    /// Reload this ArcShiftLight-instance.
    /// This allows dropping heap blocks kept alive by this instance of
    /// ArcShiftLight to be dropped.
    pub fn reload(&mut self) {
        let mut strength = 1;
        loop {
            debug_println!(
                "ArcShiftLight::reload {:?}, strength {}",
                self.item,
                strength
            );
            let Some(next) = Self::load_nontentative_next(self.item.as_ptr()) else {
                atomic::spin_loop();
                continue;
            };
            debug_println!("ArcShiftLight::reload, next = {:?}", next);
            if undecorate(next).is_null() {
                if strength > 1 {
                    let count =
                        get_refcount(self.item.as_ptr()).fetch_sub(MAX_ROOTS - 1, Ordering::SeqCst);
                    debug_println!(
                        "ArcShiftLight::reload, next = {:?}, adjusting count {} -> {}",
                        self.item,
                        count,
                        count.wrapping_sub(MAX_ROOTS - 1)
                    );
                    assert!(count >= MAX_ROOTS);
                }
                if strength == 0 {
                    let count = get_refcount(self.item.as_ptr()).fetch_add(1, Ordering::SeqCst);
                    debug_println!(
                        "ArcShiftLight::reload2b, next = {:?}, adjusting count {} -> {}",
                        self.item,
                        count,
                        count.wrapping_sub(MAX_ROOTS - 1)
                    );
                    assert!(count >= 1);
                }
                break;
            }

            if strength > 0 {
                let count = get_refcount(self.item.as_ptr()).fetch_sub(strength, Ordering::SeqCst);
                debug_println!(
                    "ArcShiftLight::reload, next = {:?}, releasing {} -> {}",
                    next,
                    count,
                    count.wrapping_sub(strength)
                );
                assert!(count >= strength);
                if count == strength {
                    debug_println!("ArcShiftLight::reload - dropping {:?}", self.item);
                    if drop_payload_and_holder(self.item.as_ptr()) {
                        strength = MAX_ROOTS;
                    } else {
                        strength = 1;
                    }
                } else {
                    strength = 0;
                }
            }

            // SAFETY:
            // next cannot be null, because of the if-statement further up
            self.item = unsafe { NonNull::new_unchecked(undecorate(next) as *mut _) };
        }
    }

    #[cfg_attr(test, mutants::skip)]
    fn verify_count(count: usize) {
        if (count & (MAX_ROOTS - 1)) >= MAX_ROOTS - 1 {
            panic!(
                "Max limit of ArcShiftLight clones ({}) was reached",
                MAX_ROOTS
            );
        }
    }

    /// Update the contents of this ArcShift, and all other instances cloned from this
    /// instance. The next time such an instance of ArcShift is dereferenced, this
    /// new value will be returned.
    ///
    /// This method never blocks, it will return quickly.
    ///
    /// WARNING!
    /// Calling this function does *not* cause the old value to be dropped before
    /// the new value is stored. The old instance of T is dropped when the last
    /// ArcShift instance is dropped or reloaded.
    pub fn update_shared(&self, new_payload: T) {
        let item = ItemHolder {
            the_size: NoMeta,
            #[cfg(feature = "validate")]
            magic1: std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111),
            #[cfg(feature = "validate")]
            magic2: std::sync::atomic::AtomicU64::new(0x1234123412348111),
            payload: new_payload,
            next_and_state: atomic::AtomicPtr::default(),
            refcount: atomic::AtomicUsize::new(MAX_ROOTS),
        };
        let new_ptr = Box::into_raw(Box::new(item));
        ArcShift::update_shared_impl(
            self.item.as_ptr(),
            new_ptr as *mut ItemHolderDummy<T>,
            true,
            null(),
        );
    }

    /// Retrieves the internal node count, counted from this instance.
    #[allow(unused)]
    #[cfg_attr(test, mutants::skip)] //Only used for tests, doesn't need cargo mutants
    fn get_internal_node_count(&self) -> usize {
        let count = 1;
        let mut curitem = get_ptr(self.item);
        loop {
            let next = get_next_and_state(curitem).load(Ordering::Relaxed);
            if is_superseded_by_tentative(get_state(next)) {
                return count + 1;
            }
            curitem = undecorate(next);
            if curitem.is_null() {
                return count;
            }
        }
    }

    /// Update the contents of this ArcShiftLight, and all other instances cloned from this
    /// instance. The next time such an instance of ArcShift is dereferenced, this
    /// new value will be returned.
    ///
    /// This method never blocks, it will return quickly.
    ///
    /// WARNING!
    /// Calling this function does *not* cause the old value to be dropped before
    /// the new value is stored. The old instance of T is dropped when the last
    /// ArcShift instance reloads to the new value. This reload happens only
    /// when the last instance is dropped or reloaded.
    ///
    /// Note, this method, in contrast to 'upgrade_shared', actually does reload
    /// the 'self' ArcShiftLight-instance. This has the effect that if 'self' is the
    /// last remaining instance, the old value that is being replaced will be dropped
    /// before this function returns.
    pub fn update(&mut self, new_payload: T) {
        let item = ItemHolder {
            the_size: NoMeta,
            #[cfg(feature = "validate")]
            magic1: std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111),
            #[cfg(feature = "validate")]
            magic2: std::sync::atomic::AtomicU64::new(0x1234123412348111),
            payload: new_payload,
            next_and_state: atomic::AtomicPtr::default(),
            refcount: atomic::AtomicUsize::new(MAX_ROOTS),
        };
        let new_ptr = Box::into_raw(Box::new(item));
        ArcShift::update_shared_impl(
            self.item.as_ptr(),
            new_ptr as *mut ItemHolderDummy<T>,
            false,
            null(),
        );
        self.reload();
    }
    /// Update the contents of this ArcShiftLight, and all other instances cloned from this
    /// instance. The next time such an instance of ArcShift is dereferenced, this
    /// new value will be returned.
    ///
    /// This method never blocks, it will return quickly.
    ///
    /// This function is useful for types T which are too large to fit on the stack.
    /// The value T will be moved directly to the internal heap-structure, without
    /// being even temporarily stored on the stack, even in debug builds.
    ///
    /// WARNING!
    /// Calling this function does *not* cause the old value to be dropped before
    /// the new value is stored. The old instance of T is dropped when the last
    /// ArcShift instance reloads to the new value. This reload happens only
    /// when the last instance is dropped or reloaded.
    ///
    /// Note, this method, in contrast to 'upgrade_shared', actually does reload
    /// the 'self' ArcShiftLight-instance. This has the effect that if 'self' is the
    /// last remaining instance, the old value that is being replaced will be dropped
    /// before this function returns.
    pub fn update_box(&mut self, new_payload: Box<T>) {
        let new_ptr = ArcShift::from_box_impl(new_payload);
        ArcShift::update_shared_impl(self.item.as_ptr(), new_ptr as *mut _, false, null());
        self.reload();
    }

    /// Update the contents of this ArcShift, and all other instances cloned from this
    /// instance. The next time such an instance of ArcShift is dereferenced, this
    /// new value will be returned.
    ///
    /// This method never blocks, it will return quickly.
    ///
    /// This function is useful for types T which are too large to fit on the stack.
    /// The value T will be moved directly to the internal heap-structure, without
    /// being even temporarily stored on the stack, even in debug builds.
    ///
    /// WARNING!
    /// Calling this function does *not* cause the old value to be dropped before
    /// the new value is stored. The old instance of T is dropped when the last
    /// ArcShift instance is dropped or reloaded.
    pub fn update_shared_box(&self, new_payload: Box<T>) {
        let new_ptr = ArcShift::from_box_impl(new_payload);
        ArcShift::update_shared_impl(self.item.as_ptr(), new_ptr as *mut _, true, null());
    }

    /// Create an ArcShift instance from this ArcShiftLight.
    pub fn upgrade(&self) -> ArcShift<T> {
        debug_println!("ArcShiftLight Promoting to ArcShift {:?}", self.item);
        let mut curitem = self.item.as_ptr();
        loop {
            let Some(next) = Self::load_nontentative_next(curitem) else {
                atomic::spin_loop();
                continue;
            };

            debug_println!(
                "ArcShiftLight upgrade {:?}, next: {:?} = {:?}",
                curitem,
                next,
                get_state(next)
            );

            if !next.is_null() {
                debug_println!(
                    "ArcShiftLight traversing chain to {:?} -> {:?}",
                    curitem,
                    next
                );
                curitem = undecorate(next) as *mut _;
                atomic::spin_loop();
                continue;
            }

            let precount = get_refcount(curitem).fetch_add(MAX_ROOTS, Ordering::SeqCst);
            if precount >= MAX_ARCSHIFT {
                let _precount = get_refcount(curitem).fetch_sub(MAX_ROOTS, Ordering::SeqCst);
                panic!(
                    "Maximum supported ArcShift instance count reached: {}",
                    MAX_ARCSHIFT
                );
            }
            atomic::fence(Ordering::SeqCst); //Just to make loom work
            debug_println!(
                "Promote {:?}, prev count: {}, new count {}",
                curitem,
                precount,
                precount + MAX_ROOTS
            );
            assert!(precount >= 1);
            let Some(next) = Self::load_nontentative_next(curitem) else {
                let _precount = get_refcount(curitem).fetch_sub(MAX_ROOTS, Ordering::SeqCst);
                assert!(_precount > MAX_ROOTS && _precount < 1_000_000_000_000);
                atomic::spin_loop();
                continue;
            };
            if !undecorate(next).is_null() {
                debug_println!(
                    "ArcShiftLight About to reduce count {:?} by {}, and traversing to {:?}",
                    curitem,
                    MAX_ROOTS,
                    next
                );

                let _precount = get_refcount(curitem).fetch_sub(MAX_ROOTS, Ordering::SeqCst);
                assert!(_precount > MAX_ROOTS && _precount < 1_000_000_000_000);
                curitem = undecorate(next) as *mut _;
                atomic::spin_loop();
                continue;
            }

            let mut temp = ArcShift { item:
                // SAFETY:
                // curitem cannot be null, since it is either the value of 'self.item', which
                // cannot be null, or assigned by code above that first checks for null
                unsafe { NonNull::new_unchecked(curitem) }
            };
            temp.reload();
            return temp;
        }
    }
}

impl<T: 'static + ?Sized> Drop for ArcShiftLight<T> {
    fn drop(&mut self) {
        debug_println!("ArcShiftLight::drop: {:?}", self.item);
        drop_root_item(get_ptr(self.item), 1)
    }
}

/// SAFETY:
/// If `T` is `Sync`, `ArcShift<T>` can also be `Sync`
unsafe impl<T: 'static + Sync> Sync for ArcShift<T> {}

/// SAFETY:
/// If `T` is `Send`, `ArcShift<T>` can also be `Send`
unsafe impl<T: 'static + Send> Send for ArcShift<T> {}

impl<T: ?Sized> Drop for ArcShift<T> {
    fn drop(&mut self) {
        verify_item(get_ptr(self.item));

        self.reload();
        debug_println!("ArcShift::drop({:?}) - reloaded", self.item);
        drop_item(get_ptr(self.item));
        debug_println!("ArcShift::drop({:?}) DONE", self.item);
    }
}

// ItemHolderDummy-pointers always point at the start of an ItemHolder object.
// It is possible to access next_and_state and refcount without knowing if the ItemHolder
// has fixed size.
#[repr(transparent)]
struct ItemHolderDummy<T: 'static + ?Sized>(u8, PhantomData<T>);

struct NoMeta;
trait IMetadata {}
impl IMetadata for NoMeta {}
impl<T: ?Sized> IMetadata for Metadata<T> {}

/// Align 4 is needed, since we store flags in the lower 2 bits of the ItemHolder-pointers
/// In practice, the alignment of ItemHolder is 8 anyway, but we specify it here for clarity.
#[repr(align(8))]
#[repr(C)] // Just to get the 'magic' first and last in memory. Shouldn't hurt.
struct ItemHolder<T: 'static + ?Sized, M: IMetadata> {
    the_size: M,
    #[cfg(feature = "validate")]
    magic1: std::sync::atomic::AtomicU64,
    next_and_state: atomic::AtomicPtr<ItemHolderDummy<T>>,
    refcount: atomic::AtomicUsize,
    #[cfg(feature = "validate")]
    magic2: std::sync::atomic::AtomicU64,

    payload: T, //TODO: We should use ManuallyDrop<T> here. It would simplify the code (but not affect correctness)
}

#[cfg(all(any(loom, feature = "shuttle"), feature = "validate"))]
static MAGIC: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);

impl<T: 'static + ?Sized, M: IMetadata> ItemHolder<T, M> {
    #[cfg_attr(test, mutants::skip)]
    #[cfg(feature = "validate")]
    fn verify(ptr2: *const ItemHolderDummy<T>) {
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
                let magic = MAGIC.fetch_add(1, Ordering::Relaxed);
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
            verify_item_impl::<T, NoMeta>(_ptr)
        } else {
            verify_item_impl::<T, Metadata<T>>(_ptr)
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
    #[allow(unused)]
    Undecorated = 0,
    /// The item is no longer current.
    /// Its payload is still not dropped.
    /// There is a next value.
    /// The next value has not yet actually been used, and may be supplanted by
    /// yet another next value.
    SupersededByTentative = 1,
    /// The item is no longer current.
    /// Its payload has still not been dropped.
    /// There is a next value.
    /// The next value has been put into use, and the payload of the ItemHolder of
    /// next will not be replaced (but of course dropped at some point)
    Superseded = 2,
    /// The item is no longer current.
    /// The payload *has* been dropped.
    /// There is a next value.
    Dropped = 3,
}

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

/// Panic if the pointer is decorated
#[cfg_attr(test, mutants::skip)]
#[inline]
fn assert_is_undecorated<T: ?Sized>(_ptr: *const ItemHolderDummy<T>) {
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
    if raw != 0 {
        ((cand as *const u8).wrapping_offset(-(raw as isize))) as *const ItemHolderDummy<T>
    } else {
        cand
    }
}

/// Get the state encoded in the decoration, if any.
/// Returns None if the pointer is undecorated null.
/// The pointer must be valid.
fn get_state<T: ?Sized>(ptr: *const ItemHolderDummy<T>) -> Option<ItemStateEnum> {
    if ptr.is_null() {
        return None;
    }
    let raw = ((ptr as usize) & 3) as u8;
    #[cfg(feature = "validate")]
    if raw == 0 {
        panic!(
            "Internal error in ArcShift: Encountered undecorated pointer in get_state!: {:?}",
            ptr
        );
    }
    // SAFETY:
    // All values `0..=3` are valid ItemStateEnum.
    // And the bitmask produces a value `0..=3`
    Some(unsafe { std::mem::transmute::<u8, ItemStateEnum>(raw) })
}
/// Returns true if the given option is Some(Dropped)
fn is_dropped(state: Option<ItemStateEnum>) -> bool {
    matches!(state, Some(ItemStateEnum::Dropped))
}
/// Returns true if the given option is Some(SupersededByTentative)
fn is_superseded_by_tentative(state: Option<ItemStateEnum>) -> bool {
    matches!(state, Some(ItemStateEnum::SupersededByTentative))
}

impl<T: 'static> Clone for ArcShift<T> {
    fn clone(&self) -> Self {
        debug_println!("ArcShift::clone({:?})", self.item);
        let rescount =
            get_refcount(self.item.as_ptr()).fetch_add(MAX_ROOTS, atomic::Ordering::Relaxed);

        debug_println!(
            "Clone - adding count to {:?}, resulting in count {}",
            self.item,
            rescount + MAX_ROOTS
        );
        if rescount >= MAX_ARCSHIFT {
            get_refcount(self.item.as_ptr()).fetch_sub(MAX_ROOTS, atomic::Ordering::SeqCst);
            panic!("Internal error in ArcShift: Max number of ArcShift instances exceeded");
        }
        ArcShift { item: self.item }
    }
}
impl<T> Deref for ArcShift<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.shared_get()
    }
}

// ptr must be a valid pointer
#[inline(always)]
fn get_next_and_state<'a, T: ?Sized>(
    ptr: *const ItemHolderDummy<T>,
) -> &'a atomic::AtomicPtr<ItemHolderDummy<T>> {
    if is_sized::<T>() {
        let offset = mem::offset_of!(ItemHolder<T,NoMeta>, next_and_state) as isize;
        // SAFETY:
        // 'offset' is a valid offset into the object, so the 'offset'-call is sound.
        let retptr = unsafe { (ptr as *const u8).offset(offset) }
            as *const atomic::AtomicPtr<ItemHolderDummy<T>>;
        // SAFETY:
        // retptr is a valid pointer
        unsafe { &*retptr }
    } else {
        let offset = mem::offset_of!(ItemHolder<T,Metadata<T>>, next_and_state) as isize;
        // SAFETY:
        // 'offset' is a valid offset into the object, so the 'offset'-call is sound.
        let retptr = unsafe { (ptr as *const u8).offset(offset) }
            as *const atomic::AtomicPtr<ItemHolderDummy<T>>;
        // SAFETY:
        // retptr is a valid pointer
        unsafe { &*retptr }
    }
}
// ptr must be a valid pointer

#[inline(always)]
fn get_refcount<'a, T: ?Sized>(ptr: *const ItemHolderDummy<T>) -> &'a atomic::AtomicUsize {
    if is_sized::<T>() {
        let offset = mem::offset_of!(ItemHolder<T,NoMeta>, refcount) as isize;
        let retptr = (ptr as *const u8).wrapping_offset(offset) as *const atomic::AtomicUsize;
        // SAFETY:
        // retptr is a valid pointer
        unsafe { &*retptr }
    } else {
        let offset = mem::offset_of!(ItemHolder<T,Metadata<T>>, refcount) as isize;
        let retptr = (ptr as *const u8).wrapping_offset(offset) as *const atomic::AtomicUsize;
        // SAFETY:
        // retptr is a valid pointer
        unsafe { &*retptr }
    }
}
impl<T: 'static> ArcShift<T> {
    /// Drop the given pointer, and return Some(T) if it was the last
    /// reference ot a non-dropped value.
    /// This only returns Some if self is the last object keeping the references
    /// value alive.
    fn drop_impl_ret(mut self_item: *const ItemHolderDummy<T>) -> Option<T> {
        verify_item(self_item);
        if self_item.is_null() {
            return None;
        }
        let mut strength = MAX_ROOTS;
        loop {
            debug_println!("drop_impl_ret on {:?}", self_item);
            debug_println!(
                "drop_impl_ret to reduce count {:?} by {}",
                self_item,
                MAX_ROOTS
            );
            let count = get_refcount(self_item).fetch_sub(strength, atomic::Ordering::SeqCst);
            debug_println!(
                "drop_impl_ret on {:?} - count: {} -> {}",
                self_item,
                count,
                count.wrapping_sub(strength)
            );
            assert!(count >= strength);
            if count == strength {
                debug_println!("drop_impl_ret decided to drop {:?}", self_item);
                let new_item = undecorate(get_next_and_state(self_item).load(Ordering::SeqCst));
                verify_item(new_item);
                verify_item(self_item);
                if new_item.is_null() {
                    //TODO: asdf
                    // SAFETY:
                    // self_item is always a valid pointer
                    let mut val = unsafe {
                        Box::from_raw(self_item as *mut MaybeUninit<ItemHolder<T, NoMeta>>)
                    };
                    // SAFETY:
                    // `val` is a valid box. The pointer created by as_mut_ptr is also valid.
                    // We then 'read' the payload. This is safe, we've marked the contents of self_item as
                    // dropped and nothing will ever access (or drop) payload later.
                    let payload =
                        unsafe { ((&mut { &mut *val.as_mut_ptr() }.payload) as *mut T).read() };
                    return Some(payload);
                }
                if drop_payload_and_holder(self_item) {
                    strength = MAX_ROOTS;
                } else {
                    strength = 1;
                }
                self_item = new_item;
                atomic::spin_loop();
            } else {
                debug_println!("No dropping_ret {:?}, exiting drop-loop", self_item);
                return None;
            }
        }
    }
    /// Try to move the value out of the ArcShift instance.
    /// This only succeeds if the self instance is the only instance
    /// holding the value. Any other instances of ArcShift or ArcShiftLight holding the
    /// same value will cause this method to return None.
    pub fn try_into_inner(mut self) -> Option<T> {
        self.reload();

        debug_println!("---- Try_into_inner ----");
        let retval = Self::drop_impl_ret(self.item.as_ptr());
        std::mem::forget(self);
        retval
    }

    /// This method calls the supplied function 'f' with the previous value pointed to as an
    /// argument. It then sets the pointee value to the value returned by the function.
    ///
    /// Note, there is no guarantee that the ArcShift value isn't updated while the supplied function
    /// 'f' is running. This will be detected, and the value returned by 'f' will
    /// be dropped and no update will occur. This function returns (true, _ ) if the value
    /// was updated, otherwise it returns (false, _);
    ///
    /// If other threads do simultaneous updates, there's no particular guarantee
    /// that whatever value is returned by f will ever be read by another thread before it is
    /// overwritten, unless the other writes also use the 'rcu'/'rcu_x'-functions.
    ///
    /// This method never blocks, it will return quickly (depending on the execution time
    /// of 'f').
    ///
    /// The second value of the returned tuple is simply the value produced by the function
    /// 'projector', which is called with the current pointee value after any update has occurred.
    ///
    /// WARNING!
    /// Calling this function does *not* cause the old value to be dropped before
    /// the new value is stored. The old instance of T is dropped when the last
    /// ArcShift instance reloads to the new value. This reload happens only
    /// when the last instance is dropped or reloaded.
    ///
    /// Note, this method, in contrast to (for example) 'upgrade_shared', actually does reload
    /// the 'self' ArcShift-instance. This has the effect that *if* 'self' is the
    /// last remaining instance, the old value that is being replaced *will* be dropped
    /// before this function returns.
    pub fn rcu_project<'s, A>(
        &'s mut self,
        f: impl FnOnce(&T) -> Option<T>,
        projector: impl FnOnce(&'s T) -> A,
    ) -> (bool, A) {
        let ret = match self.rcu_impl(|old| f(old)) {
            RcuResult::Update => true,
            RcuResult::NoUpdate | RcuResult::Race => false,
        };
        (ret, projector(self.shared_non_reloading_get()))
    }

    /// This method calls the supplied function with the previous value pointed to.
    /// It then sets the current value to the value returned by the function, if the function
    /// returns Some(...). If the function returns None, there is no effect and 'rcu_maybe'
    /// returns false.
    ///
    /// Note, there is no guarantee that the value isn't updated while the supplied function
    /// 'f' is running. This will be detected, and the value returned by 'f' will
    /// be dropped and no update will occur. This function returns true if the value
    /// was updated, false otherwise. The only reason it can fail is if some other thread
    /// updated the value while 'rcu_maybe' was running.
    ///
    /// If other threads do simultaneous updates, there's no particular guarantee
    /// that whatever value is returned by f will ever be read by another thread before it is
    /// overwritten, unless the other writes also use the 'rcu'/'rcu_x'-functions.
    ///
    /// This method never blocks, it will return quickly (depending on the execution time
    /// of 'f').
    ///
    /// WARNING!
    /// Calling this function does *not* cause the old value to be dropped before
    /// the new value is stored. The old instance of T is dropped when the last
    /// ArcShift instance reloads to the new value. This reload happens only
    /// when the last instance is dropped or reloaded.
    ///
    /// Note, this method, in contrast to (for example) 'upgrade_shared', actually does reload
    /// the 'self' ArcShift-instance. This has the effect that if 'self' is the
    /// last remaining instance, the old value that is being replaced will be dropped
    /// before this function returns.
    pub fn rcu_maybe<F>(&mut self, f: F) -> bool
    where
        F: FnOnce(&T) -> Option<T>,
    {
        match self.rcu_impl(f) {
            RcuResult::Update => true,
            RcuResult::NoUpdate | RcuResult::Race => false,
        }
    }
}
impl<T: 'static + ?Sized> ArcShift<T> {
    fn simple_early_drop_opt(cand: *const ItemHolderDummy<T>) -> Option<bool> {
        verify_item(cand);
        let count = get_refcount(cand).load(Ordering::SeqCst);

        verify_item(cand);
        let Some(next_ptr) = ArcShiftLight::load_nontentative_next(cand) else {
            atomic::spin_loop();
            return None;
        };
        if (2..MAX_ROOTS).contains(&count) {
            debug_println!(
                "Possibility of early drop for {:?}! (count = {}, state: {:?})",
                cand,
                count,
                get_state(next_ptr)
            );

            if !undecorate(next_ptr).is_null()
                && get_state(next_ptr) != Some(ItemStateEnum::Dropped)
            {
                debug_println!("First condition for early drop fulfilled: {:?}", next_ptr);
                match get_next_and_state(cand).compare_exchange(
                    next_ptr as *mut _,
                    decorate(next_ptr, Dropped) as *mut _,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => {
                        let count = get_refcount(cand).load(Ordering::SeqCst);
                        debug_println!("Early drop count: {:?} : {}", next_ptr, count);
                        if count >= MAX_ROOTS {
                            debug_println!("Simultaneous count increase detected {:?}", cand);
                            // Ok, someone must have increased the refcount just before we did compare_exchange. Undo the compare_exchange
                            match get_next_and_state(cand).compare_exchange(
                                decorate(next_ptr, Dropped) as *mut _,
                                decorate(next_ptr, Superseded) as *mut _,
                                Ordering::SeqCst,
                                Ordering::SeqCst,
                            ) {
                                Ok(_) => {
                                    debug_println!("Spinning on {:?} drop sync(1)", cand);
                                    atomic::spin_loop();
                                    None
                                }
                                Err(_) => {
                                    debug_println!("Spinning on {:?} drop sync(2)", cand);
                                    atomic::spin_loop();
                                    None
                                }
                            }
                        } else {
                            // Ok, we're the last owner. We can drop the payload
                            verify_item(cand);

                            // SAFETY:
                            // `cand` is a valid pointer, which we are allowed to drop the
                            // payload for.
                            let payload_item_mut = unsafe {
                                if is_sized::<T>() {
                                    let full_ptr = get_full_ptr_raw::<T, NoMeta>(cand);
                                    &mut *addr_of_mut!(
                                        (*(full_ptr as *mut ItemHolder<T, NoMeta>)).payload
                                    )
                                } else {
                                    let full_ptr = get_full_ptr_raw::<T, Metadata<T>>(cand);
                                    &mut *addr_of_mut!(
                                        (*(full_ptr as *mut ItemHolder<T, Metadata<T>>)).payload
                                    )
                                }
                            };
                            // SAFETY:
                            // `payload_item_mut` is a valid pointer to the payload, that
                            // we are allowed to drop.
                            unsafe { std::ptr::drop_in_place(payload_item_mut) }
                            debug_println!("Performed early drop for {:?}", cand);
                            Some(true)
                        }
                    }
                    Err(_) => {
                        debug_println!("Spinning on {:?} drop sync(3)", cand);
                        atomic::spin_loop();
                        None
                    }
                }
            } else {
                debug_println!(
                    "However, first condition for early drop not fulfilled: {:?}",
                    next_ptr
                );
                Some(false)
            }
        } else {
            debug_println!(
                "No early drop of {:?}, count (={}) not permitting",
                cand,
                count
            );
            Some(false)
        }
    }
    fn drop_impl(mut self_item: *const ItemHolderDummy<T>) {
        verify_item(self_item);
        let mut strength = MAX_ROOTS;
        debug_println!(
            "start of drop_impl on {:?}, strength: {}",
            self_item,
            strength
        );
        loop {
            debug_println!("drop_impl on {:?}", self_item);
            if self_item.is_null() {
                return;
            }

            debug_println!("drop_impl to reduce count {:?} by {}", self_item, strength);
            let count = get_refcount(self_item).fetch_sub(strength, atomic::Ordering::SeqCst);
            debug_println!(
                "drop_impl on {:?} - count: {} -> {}",
                self_item,
                count,
                count.wrapping_sub(strength)
            );
            assert!(count >= strength);
            if count == strength {
                debug_println!("drop_impl decided to drop {:?}", self_item);
                let new_item = undecorate(get_next_and_state(self_item).load(Ordering::SeqCst));

                verify_item(new_item);
                verify_item(self_item);
                if drop_payload_and_holder(self_item) {
                    strength = MAX_ROOTS;
                } else {
                    strength = 1;
                }
                self_item = new_item;
                atomic::spin_loop();
            } else {
                debug_println!("No dropping {:?}, exiting drop-loop", self_item);
                return;
            }
        }
    }
    /// This function makes sure to update this instance of ArcShift to the newest value.
    ///
    /// Calling the regular [`ArcShift::get`] already does this, so this is rarely needed.
    /// But if mutable access to a ArcShift is only possible at certain points in the program,
    /// it may be clearer to call `reload` at those points to ensure any updates take
    /// effect, compared to just calling 'get' and discarding the value.
    #[inline(never)]
    pub fn reload(&mut self) {
        self.reload_impl(null());
    }
    fn reload_impl(&mut self, limit: *const ItemHolderDummy<T>) {
        verify_item(self.item.as_ptr());
        assert_is_undecorated(limit);
        debug_println!(
            "reload {:?} (next = {:?})",
            self.item,
            get_next_and_state(self.item.as_ptr())
        );

        let mut new_self = get_ptr(self.item);
        verify_item(new_self);
        loop {
            verify_item(new_self);

            let Some(next) = ArcShiftLight::load_nontentative_next(new_self) else {
                atomic::spin_loop();
                continue;
            };
            assert_is_undecorated(new_self);
            if next.is_null() || new_self == limit {
                if new_self == self.item.as_ptr() {
                    debug_println!("{:?} doesn't need reload", self.item);
                    return; //Nothing to do
                }

                let dbgprev = get_refcount(new_self).fetch_add(MAX_ROOTS, Ordering::SeqCst);
                if dbgprev > MAX_ARCSHIFT {
                    get_refcount(new_self).fetch_sub(MAX_ROOTS, atomic::Ordering::SeqCst);
                    panic!("Maximum ArcShift instance count reached");
                }
                atomic::fence(Ordering::SeqCst); //Just for loom
                debug_println!(
                    "{:?} has no next - adding count: {} -> {}. (our strongly owned item: {:?})",
                    new_self,
                    dbgprev,
                    dbgprev + MAX_ROOTS,
                    self.item
                );

                // We're in ArcShift, and we transitively have a reference to it!
                assert_ne!(dbgprev, 0); // This is guaranteed, since we're still holding a count on self.item, which has a chain all the way to 'new_self'.

                break;
            }

            verify_item(undecorate(next));
            let _count = get_refcount(undecorate(next)).load(Ordering::SeqCst);
            debug_println!(
                "Moving from {:?} to {:?} ({:?} has count {})",
                new_self,
                undecorate(next),
                undecorate(next),
                _count
            );
            new_self = undecorate(next);
            verify_item(new_self);
            atomic::spin_loop();
        }

        let mut cand = self.item.as_ptr() as *const _;
        let mut strength = MAX_ROOTS;
        loop {
            assert_is_undecorated(cand);
            assert_is_undecorated(new_self);
            if cand == new_self {
                // We're already carrying a refcount from before the loop.
                // This is needed so nothing drops the payload while we're looping.
                // But if we get to here, we've inherited the refcount from the previous owner.
                // So we need to subtract this.
                debug_println!(
                    "Reached new_self, reload about to reduce count {:?} by {}",
                    new_self,
                    strength
                );
                let _t = get_refcount(new_self).fetch_sub(strength, Ordering::SeqCst);
                assert!(_t > strength);
                debug_println!(
                    "Reached new_self (={:?}), reducing count {} -> {}",
                    new_self,
                    _t,
                    _t - strength
                );
                break;
            }
            verify_item(cand);

            if strength > 1 {
                debug_println!("reload(3) to reduce count {:?} by {}", cand, MAX_ROOTS - 1);
                let count = get_refcount(cand).fetch_sub(MAX_ROOTS - 1, Ordering::SeqCst);
                debug_println!(
                    "{:?} release refcount {} -> {}",
                    cand,
                    count,
                    count.wrapping_sub(MAX_ROOTS - 1)
                );
                assert!(count >= MAX_ROOTS - 1);
                if count == MAX_ROOTS {
                    debug_println!("Dropping payload of {:?}", cand);
                    let newcand = undecorate(get_next_and_state(cand).load(Ordering::SeqCst));

                    // NOTE: Because we now uniquely own 'cand', nothing can do an 'anti-garbage' drop of
                    // a tentative 'cand.next_and_state', since that would require access to 'cand', which no
                    // one can have.
                    verify_item(newcand);
                    verify_item(cand);

                    let strong = drop_payload_and_holder(cand);
                    if strong {
                        strength = MAX_ROOTS;
                    } else {
                        strength = 1;
                    }
                    cand = newcand;
                    continue;
                }
            }

            {
                let Some(early_dropped) = Self::simple_early_drop_opt(cand) else {
                    if strength > 1 {
                        let _dbg = get_refcount(cand).fetch_add(MAX_ROOTS - 1, Ordering::SeqCst);
                        debug_println!(
                            "{:?} release refcount race, undoing {} -> {}",
                            cand,
                            _dbg,
                            _dbg.wrapping_add(MAX_ROOTS - 1)
                        );
                        assert!(_dbg > 0);
                    }
                    atomic::spin_loop();
                    continue;
                };
                let newcand = undecorate(get_next_and_state(cand).load(Ordering::SeqCst));
                if early_dropped {
                    let _dbg = get_refcount(newcand).fetch_add(1, Ordering::SeqCst);
                    debug_println!(
                        "Early drop, creating strong+weak ref to {:?} by going {} -> {}",
                        newcand,
                        _dbg,
                        _dbg.wrapping_add(1)
                    );
                }

                let count = get_refcount(cand).fetch_sub(1, Ordering::SeqCst);
                assert!(count >= 1);
                debug_println!(
                    "early-drop, no-drop-case reducing {} -> {} for {:?}",
                    count,
                    count.wrapping_sub(1),
                    cand
                );

                if count == 1 {
                    // NOTE: Because we now uniquely own 'cand', nothing can do an 'anti-garbage' drop of
                    // a tentative 'cand.next_and_state', since that would require access to 'cand', which no
                    // one can have.
                    verify_item(newcand);
                    verify_item(cand);

                    let strong = drop_payload_and_holder(cand);
                    if strong {
                        if early_dropped {
                            unreachable!("Impossible condition");
                        } else {
                            // Nothing needs to be done, we have inherited a strong 'newcand'-reference
                        }
                        strength = MAX_ROOTS;
                    } else if early_dropped {
                        let _dbg = get_refcount(newcand).fetch_sub(1, Ordering::SeqCst);
                        assert!(_dbg > 1);
                        debug_println!(
                            "Adjusting[1] ref to {:?} by going {} -> {}",
                            newcand,
                            _dbg,
                            _dbg.wrapping_sub(1)
                        );
                        strength = MAX_ROOTS;
                    } else {
                        debug_println!("Adjusting[2] ref to {:?} ", newcand);
                        strength = 1;
                    }

                    debug_println!("Racy final drop of {:?}, proceeding to {:?}", cand, newcand);
                    cand = newcand;
                    continue;
                }
                if early_dropped {
                    cand = newcand;
                    strength = MAX_ROOTS;
                    continue;
                }

                // At this point, we know _nothing_ about cand.
                // It can even have been dropped.
                debug_println!("Exiting drop-loop at {:?}", cand);

                // We've already increased the refcount on new_self.
                break;
            }
        }

        debug_println!("Reload moving {:?} -> {:?}", self.item, new_self);
        // SAFETY:
        // new_self cannot be null, since it is either self.item (not null), or
        // has been assigned by code which first checked for null.
        self.item = unsafe { NonNull::new_unchecked(new_self as *mut _) };
    }
    /// The returned pointer is never null. Even for zero-sized T.
    fn from_box_impl(input: Box<T>) -> *const ItemHolderDummy<T> {
        if is_sized::<T>() {
            make_sized_holder_from_box(input)
        } else {
            make_unsized_holder_from_box(input)
        }
    }
    /// Basically the same as doing [`ArcShift::new`], but avoids copying the contents of 'input'
    /// to the stack, even as a temporary variable. This can be useful, if the type is too large
    /// to fit on the stack.
    pub fn from_box(input: Box<T>) -> ArcShift<T> {
        ArcShift {
            // SAFETY:
            // from_box_impl never creates a null-pointer from a Box.
            item: unsafe { NonNull::new_unchecked(Self::from_box_impl(input) as *mut _) },
        }
    }

    #[cold]
    fn shared_get_impl(&self) -> *const ItemHolderDummy<T> {
        let mut next_self_item = self.item.as_ptr() as *const _;
        loop {
            debug_println!("shared_get_impl loop {:?}", next_self_item);
            assert_is_undecorated(next_self_item);
            verify_item(next_self_item);
            let Some(cand) = ArcShiftLight::load_nontentative_next(next_self_item) else {
                atomic::spin_loop();
                continue;
            };
            if !cand.is_null() {
                debug_println!("Doing assign");
                next_self_item = undecorate(cand);
            } else {
                break;
            }
        }
        next_self_item
    }

    /// Like 'get', but only requires a &self, not &mut self.
    ///
    /// WARNING!
    /// This does not free old values after update. Call `ArcShift::get` or `ArcShift::reload` to ensure this.
    /// Also, the run time of this method is proportional to the number of updates which have
    /// taken place without a 'get' or 'reload'. The overhead is basically the price of
    /// one memory access per new available version - a few nanoseconds per version if data is in
    /// CPU cache.
    ///
    /// Because of this, it is not advisable to do an unbounded number of updates, if
    /// ArcShift instances exist that only use 'shared_get', and never do reloads.
    #[inline]
    pub fn shared_get(&self) -> &T {
        debug_println!("Getting {:?}", self.item);
        // SAFETY:
        // `self.item` is always a valid pointer
        let cand = get_next_and_state(self.item.as_ptr()).load(atomic::Ordering::Relaxed);
        if !cand.is_null() {
            debug_println!("Update to {:?} detected", cand);
            if is_sized::<T>() {
                // SAFETY:
                // the pointer returned by shared_get_impl is always valid.
                return &unsafe { &*get_full_ptr_raw::<T, NoMeta>(self.shared_get_impl()) }.payload;
            } else {
                // SAFETY:
                // the pointer returned by shared_get_impl is always valid.
                return &unsafe { &*get_full_ptr_raw::<T, Metadata<T>>(self.shared_get_impl()) }
                    .payload;
            }
        }
        debug_println!("Returned payload for {:?}", self.item);
        if is_sized::<T>() {
            // SAFETY:
            // `self.item` is always a valid pointer
            &unsafe { &*get_full_ptr::<T, NoMeta>(self.item) }.payload
        } else {
            // SAFETY:
            // `self.item` is always a valid pointer
            &unsafe { &*get_full_ptr::<T, Metadata<T>>(self.item) }.payload
        }
    }

    /// Return the value pointed to.
    ///
    /// This method is very fast, basically the speed of a regular reference, unless
    /// the value has been modified by calling one of the update-methods.
    ///
    /// Note that this method requires `mut self`. The reason 'mut' self is needed, is because
    /// this allows us to update the pointer when new values become available after modification.
    /// Without unique access (given by `&mut self`), we can't know what references `&T` to the old
    /// value still remain alive.
    #[inline]
    pub fn get(&mut self) -> &T {
        debug_println!("Getting {:?}", self.item);
        let cand = get_next_and_state(self.item.as_ptr()).load(atomic::Ordering::Relaxed);
        if !cand.is_null() {
            debug_println!("Update to {:?} detected", cand);
            self.reload();
        }
        debug_println!("Returned payload for {:?}", self.item);
        if is_sized::<T>() {
            // SAFETY:
            // `self.item` is always a valid pointer
            &unsafe { &*get_full_ptr::<T, NoMeta>(self.item) }.payload
        } else {
            // SAFETY:
            // `self.item` is always a valid pointer
            &unsafe { &*get_full_ptr::<T, Metadata<T>>(self.item) }.payload
        }
    }

    /// Try to obtain a mutable reference to the value.
    /// This only succeeds if this instance of ArcShift is the only instance
    /// of the smart pointer.
    ///
    /// Note!
    /// As a consequence of the rule above, this function will never succeed if
    /// there is a ArcShiftLight instance alive. This is because it is then possible
    /// to create another ArcShift from that ArcShiftLight, and ArcShift does not support
    /// invalidating existing instances of ArcShiftLight (or ArcShift, for that matter).
    pub fn try_get_mut(&mut self) -> Option<&mut T> {
        self.reload();

        if get_refcount(self.item.as_ptr()).load(Ordering::SeqCst) == MAX_ROOTS {
            if !undecorate(get_next_and_state(self.item.as_ptr()).load(Ordering::SeqCst)).is_null()
            {
                return None;
            }
            // SAFETY:
            // We always have refcount for self.item, and it is guaranteed valid
            if is_sized::<T>() {
                let full_ptr = get_full_ptr::<T, NoMeta>(self.item) as *mut ItemHolder<T, NoMeta>;
                // SAFETY:
                // full_ptr is a valid pointer
                Some(unsafe { &mut (*(full_ptr)).payload })
            } else {
                let full_ptr =
                    get_full_ptr::<T, Metadata<T>>(self.item) as *mut ItemHolder<T, Metadata<T>>;
                // SAFETY:
                // full_ptr is a valid pointer
                Some(unsafe { &mut (*(full_ptr)).payload })
            }
        } else {
            None
        }
    }
    /// Update the contents of this ArcShift, and all other instances cloned from this
    /// instance. The next time such an instance of ArcShift is dereferenced, this
    /// new value will be returned.
    ///
    /// This method never blocks, it will return quickly.
    ///
    /// This function is useful for types T which are too large to fit on the stack.
    /// The value T will be moved directly to the internal heap-structure, without
    /// being even temporarily stored on the stack, even in debug builds.
    ///
    /// WARNING!
    /// Calling this function does *not* cause the old value to be dropped before
    /// the new value is stored. The old instance of T is dropped when the last
    /// ArcShift instance is dropped or reloaded.
    pub fn update_shared_box(&self, new_payload: Box<T>) {
        let new_ptr = Self::from_box_impl(new_payload);
        Self::update_shared_impl(self.item.as_ptr(), new_ptr as *mut _, true, null());
    }

    fn update_shared_impl(
        self_item: *const ItemHolderDummy<T>,
        new_ptr: *mut ItemHolderDummy<T>,
        make_tentative: bool,
        expect_next: *const ItemHolderDummy<T>,
    ) -> bool {
        debug_println!("Upgrading {:?} -> {:?} ", self_item, new_ptr);
        verify_item(new_ptr);
        let mut candidate = self_item;
        verify_item(candidate);

        loop {
            verify_item(candidate);
            let curnext = get_next_and_state(candidate).load(Ordering::SeqCst);

            let expect = if is_superseded_by_tentative(get_state(curnext)) {
                curnext
            } else if undecorate(curnext).is_null() {
                null_mut()
            } else {
                loop {
                    let Some(early_dropped) = Self::simple_early_drop_opt(candidate) else {
                        atomic::spin_loop();
                        continue;
                    };
                    if early_dropped {
                        debug_println!(
                            "\n----------------------------------------------------------\n"
                        );
                        let _dbg = get_refcount(undecorate(curnext))
                            .fetch_sub(MAX_ROOTS - 1, Ordering::SeqCst);
                        debug_println!(
                            "Early_drop_adjust3 for {:?}.{:?} {} -> {}",
                            candidate,
                            curnext,
                            _dbg,
                            _dbg.wrapping_sub(MAX_ROOTS - 1)
                        );
                        assert!(_dbg > MAX_ROOTS - 1);
                    }
                    break;
                }
                candidate = undecorate(curnext);
                atomic::spin_loop();
                continue;
            };

            if !expect_next.is_null() {
                if expect.is_null() {
                    if candidate != expect_next {
                        return false;
                    }
                } else if expect as *const _ != expect_next {
                    return false;
                }
            }

            verify_item(candidate);
            verify_item(new_ptr);

            let new_decorated = if make_tentative {
                decorate(new_ptr, ItemStateEnum::SupersededByTentative)
            } else {
                decorate(new_ptr, ItemStateEnum::Superseded)
            };

            match get_next_and_state(candidate).compare_exchange(
                expect,
                new_decorated as *mut _,
                atomic::Ordering::SeqCst,
                atomic::Ordering::SeqCst,
            ) {
                Ok(_) => {
                    verify_item(undecorate(expect));
                    // Warning!
                    // At this point, 'new_ptr' can already have been dropped, by another simultaneous
                    // update_shared-invocation.

                    debug_println!(
                        "Did replace next of {:?} (={:?}) with {:?} ",
                        candidate,
                        expect,
                        new_decorated
                    );
                    if is_superseded_by_tentative(get_state(expect)) {
                        // If we get here, then the dummy-optimization, allowing us to reduce
                        // garbage when updates are never actually loaded, has been triggered.
                        debug_println!(
                            "Anti-garbage optimization was in effect, dropping {:?}",
                            expect
                        );
                        drop_item(undecorate(expect));
                    }
                    if !make_tentative {
                        loop {
                            let Some(early_dropped) = Self::simple_early_drop_opt(candidate) else {
                                atomic::spin_loop();
                                continue;
                            };
                            if early_dropped {
                                debug_println!("\n----------------------------------------------------------\n");
                                let _dbg = get_refcount(new_ptr)
                                    .fetch_sub(MAX_ROOTS - 1, Ordering::SeqCst);
                                debug_println!(
                                    "Early_drop_adjust for {:?}.{:?} {} -> {}",
                                    candidate,
                                    new_ptr,
                                    _dbg,
                                    _dbg.wrapping_sub(MAX_ROOTS - 1)
                                );
                                assert!(_dbg > MAX_ROOTS - 1);
                            }
                            break;
                        }
                    }

                    return true;
                }
                Err(other) => {
                    verify_item(candidate);
                    verify_item(new_ptr);
                    if !is_superseded_by_tentative(get_state(other)) {
                        loop {
                            let Some(early_dropped) = Self::simple_early_drop_opt(candidate) else {
                                atomic::spin_loop();
                                continue;
                            };
                            if early_dropped {
                                debug_println!("\n----------------------------------------------------------\n");
                                let _dbg = get_refcount(undecorate(other))
                                    .fetch_sub(MAX_ROOTS - 1, Ordering::SeqCst);
                                debug_println!(
                                    "Early_drop_adjust2 for {:?}.{:?} {} -> {}",
                                    candidate,
                                    other,
                                    _dbg,
                                    _dbg.wrapping_sub(MAX_ROOTS - 1)
                                );
                                assert!(_dbg > MAX_ROOTS - 1);
                            }
                            break;
                        }

                        verify_item(undecorate(other));
                        debug_println!("Update not complete - but advancing to {:?}", other);
                        candidate = undecorate(other);
                    } else {
                        debug_println!("Update not complete yet, spinning on {:?}", candidate);
                    }
                }
            }
            atomic::spin_loop();
        }
    }
    /// Update the contents of this ArcShift, and all other instances cloned from this
    /// instance. The next time such an instance of ArcShift is dereferenced, this
    /// new value will be returned.
    ///
    /// This method never blocks, it will return quickly.
    ///
    /// This function is useful for types T which are too large to fit on the stack.
    /// The value T will be moved directly to the internal heap-structure, without
    /// being even temporarily stored on the stack, even in debug builds.
    ///
    /// WARNING!
    /// Calling this function does *not* cause the old value to be dropped before
    /// the new value is stored. The old instance of T is dropped when the last
    /// ArcShift instance reloads to the new value. This reload happens only
    /// when the last instance is dropped or reloaded.
    ///
    /// Note, this method, in contrast to 'upgrade_shared', actually does reload
    /// the 'self' ArcShift-instance. This has the effect that if 'self' is the
    /// last remaining instance, the old value that is being replaced will be dropped
    /// before this function returns.
    pub fn update_box(&mut self, new_payload: Box<T>) {
        self.update_shared_box(new_payload);
        debug_println!("self.reload()");
        self.reload();
    }

    #[cfg_attr(test, mutants::skip)]
    fn verify_count(count: usize) {
        if (count & (MAX_ROOTS - 1)) == MAX_ROOTS - 1 {
            panic!("Maximum number of ArcShiftLight-instances has been reached.");
        }
    }

    /// Create an instance of ArcShiftLight, pointing to the same value as 'self'.
    ///
    /// WARNING!
    /// A maximum of 524287 ArcShiftLight-instances can be created for each value.
    /// An attempt to create more instances than this will fail with a panic.
    pub fn make_light(&self) -> ArcShiftLight<T> {
        let mut curitem = self.item.as_ptr() as *const _;
        loop {
            let Some(next) = ArcShiftLight::load_nontentative_next(curitem) else {
                atomic::spin_loop();
                continue;
            };

            if !next.is_null() {
                curitem = undecorate(next);
                atomic::spin_loop();
                continue;
            }

            let count = get_refcount(curitem).load(Ordering::SeqCst);

            Self::verify_count(count);

            if get_refcount(curitem)
                .compare_exchange(count, count + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                atomic::spin_loop();
                continue;
            }
            debug_println!("{:?}, changed refcount {} -> {}", curitem, count, count + 1);
            break;
        }
        // SAFETY:
        // curitem cannot be null. It is either equal to self.item, which is not null,
        // or has been assigned from next after first checking for null.
        ArcShiftLight {
            // SAFETY:
            // curitem is not null here. The exit conditions in the loop above guarantee this.
            item: unsafe { NonNull::new_unchecked(curitem as *mut _) },
        }
    }

    /// This is like 'get', but never reloads the pointer.
    /// This means that any new value supplied using one of the update methods will not be
    /// available.
    /// This method is ever so slightly faster than regular 'get'.
    ///
    /// WARNING!
    /// You should probably not be using this method.
    /// One use-case is if you can control locations where an update is possible, and arrange
    /// for '&mut self' to be available so that `ArcShift::reload` can be called at those locations.
    #[inline(always)]
    pub fn shared_non_reloading_get(&self) -> &T {
        if is_sized::<T>() {
            // SAFETY:
            // `self.item` is always a valid pointer
            &unsafe { &*get_full_ptr::<T, NoMeta>(self.item) }.payload
        } else {
            // SAFETY:
            // `self.item` is always a valid pointer
            &unsafe { &*get_full_ptr::<T, Metadata<T>>(self.item) }.payload
        }
    }
}

enum RcuResult {
    Update,
    NoUpdate,
    Race,
}

impl<T: 'static + Sized> ArcShift<T> {
    /// Create a new ArcShift instance, containing the given value.
    pub fn new(payload: T) -> ArcShift<T> {
        let item = ItemHolder {
            the_size: NoMeta,
            #[cfg(feature = "validate")]
            magic1: std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111),
            #[cfg(feature = "validate")]
            magic2: std::sync::atomic::AtomicU64::new(0x1234123412348111),
            payload,
            next_and_state: atomic::AtomicPtr::default(),
            refcount: atomic::AtomicUsize::new(MAX_ROOTS),
        };
        let cur_ptr = Box::into_raw(Box::new(item));
        ArcShift { item:
            // SAFETY:
            // cur_ptr has just been created by Box::into_raw, and cannot be null
            unsafe { NonNull::new_unchecked(cur_ptr as *mut ItemHolderDummy<T>) }
        }
    }

    /// Update the contents of this ArcShift, and all other instances cloned from this
    /// instance. The next time such an instance of ArcShift is dereferenced, this
    /// new value will be returned.
    ///
    /// This method never blocks, it will return quickly.
    ///
    /// WARNING!
    /// Calling this function does *not* cause the old value to be dropped before
    /// the new value is stored. The old instance of T is dropped when the last
    /// ArcShift instance is dropped or reloaded.
    pub fn update_shared(&self, new_payload: T) {
        let item = ItemHolder {
            the_size: NoMeta,
            #[cfg(feature = "validate")]
            magic1: std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111),
            #[cfg(feature = "validate")]
            magic2: std::sync::atomic::AtomicU64::new(0x1234123412348111),
            payload: new_payload,
            next_and_state: atomic::AtomicPtr::default(),
            refcount: atomic::AtomicUsize::new(MAX_ROOTS),
        };
        let new_ptr = Box::into_raw(Box::new(item));
        Self::update_shared_impl(
            self.item.as_ptr(),
            new_ptr as *mut ItemHolderDummy<T>,
            true,
            null(),
        );
    }

    /// Update the contents of this ArcShift, and all other instances cloned from this
    /// instance. The next time such an instance of ArcShift is dereferenced, this
    /// new value will be returned.
    ///
    /// This method never blocks, it will return quickly.
    ///
    /// WARNING!
    /// Calling this function does *not* cause the old value to be dropped before
    /// the new value is stored. The old instance of T is dropped when the last
    /// ArcShift instance reloads to the new value. This reload happens only
    /// when the last instance is dropped or reloaded.
    ///
    /// Note, this method, in contrast to 'upgrade_shared', actually does reload
    /// the 'self' ArcShift-instance. This has the effect that if 'self' is the
    /// last remaining instance, the old value that is being replaced will be dropped
    /// before this function returns.
    pub fn update(&mut self, new_payload: T) {
        self.update_shared(new_payload);
        debug_println!("self.reload()");
        self.reload();
    }

    /// This method calls the supplied function with the previous value pointed to.
    /// It then sets the current value to the value returned by the function.
    ///
    /// Note, there is no guarantee that the value isn't updated while the supplied function
    /// 'f' is running. This will be detected, and the value returned by 'f' will
    /// be dropped and no update will occur. This function returns true if the value
    /// was updated, false otherwise. The only reason it can fail is if some other thread
    /// updated the value while 'rcu' was running.
    ///
    /// If other threads do simultaneous updates, there's no particular guarantee
    /// that whatever value is returned by f will ever be read by another thread before it is
    /// overwritten, unless the other writes also use the 'rcu'-function.
    ///
    /// This method never blocks, it will return quickly (depending on the execution time
    /// of 'f').
    ///
    /// WARNING!
    /// Calling this function does *not* cause the old value to be dropped before
    /// the new value is stored. The old instance of T is dropped when the last
    /// ArcShift instance reloads to the new value. This reload happens only
    /// when the last instance is dropped or reloaded.
    ///
    /// Note, this method, in contrast to (for example) 'upgrade_shared', actually does reload
    /// the 'self' ArcShift-instance. This has the effect that if 'self' is the
    /// last remaining instance, the old value that is being replaced will be dropped
    /// before this function returns.
    pub fn rcu<F>(&mut self, f: F) -> bool
    where
        F: FnOnce(&T) -> T,
    {
        match self.rcu_impl(|x| Some(f(x))) {
            RcuResult::Update => true,
            RcuResult::NoUpdate | RcuResult::Race => false,
        }
    }
    fn rcu_impl<F>(&mut self, f: F) -> RcuResult
    where
        F: FnOnce(&T) -> Option<T>,
    {
        let old_ptr;
        let result: ItemHolder<T, NoMeta>;
        {
            old_ptr = self.shared_get_impl();
            let old_full_ptr = get_full_ptr_raw::<T, NoMeta>(old_ptr);
            // SAFETY:
            // old_full_ptr is a valid pointer
            let Some(payload) = f(&unsafe { &*old_full_ptr }.payload) else {
                self.reload();
                return RcuResult::NoUpdate;
            };
            result = ItemHolder {
                the_size: NoMeta,
                #[cfg(feature = "validate")]
                magic1: std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111),
                #[cfg(feature = "validate")]
                magic2: std::sync::atomic::AtomicU64::new(0x1234123412348111),
                // SAFETY:
                // old_ptr is a valid ptr
                payload,
                next_and_state: atomic::AtomicPtr::default(),
                refcount: atomic::AtomicUsize::new(MAX_ROOTS),
            };
        }

        let new_ptr = Box::into_raw(Box::new(result));
        // Note:
        // We do not run into the ABA-problem here, since both pointers that could collide
        // are distinct, valid pointers.
        let ret = Self::update_shared_impl(
            self.item.as_ptr(),
            new_ptr as *mut ItemHolderDummy<T>,
            true,
            old_ptr,
        );
        if !ret {
            // SAFETY:
            // new_ptr hasn't been used, since update_shared_impl returned false.
            // So it's still just a uniquely valid pointer created by Box::into_raw.
            unsafe { _ = Box::from_raw(new_ptr) };
            self.reload();
            RcuResult::Race
        } else {
            self.reload_impl(new_ptr as *const ItemHolderDummy<T>);
            RcuResult::Update
        }
    }
}

/// SAFETY:
/// The 'ptr' must be a valid pointer to an ItemHolder heap item that
/// is okay to drop.
fn drop_payload_and_holder<T: 'static + ?Sized>(ptr2: *const ItemHolderDummy<T>) -> bool {
    verify_item(ptr2);

    /*let fullptr = if is_sized::<T>() {
        get_full_ptr_raw::<T,NoMeta>(ptr2)
    }  else {
        get_full_ptr_raw::<T,Metadata<T>>(ptr2)
    };*/

    if is_dropped(get_state(get_next_and_state(ptr2).load(Ordering::SeqCst))) {
        debug_println!(
            "Dropping holder {:?}, but payload was already dropped",
            ptr2
        );
        // SAFETY:
        // `ptr` is always a valid pointer.
        // At this position we've established that we can drop the pointee's box memory, but the
        // pointee value has already been dropped.
        //compile_error!("This won't work, because for unsized we have effectively allocated a SizedItemHolder")
        if is_sized::<T>() {
            let fullptr = get_full_ptr_raw::<T, NoMeta>(ptr2);
            // SAFETY:
            // fullptr is a valid uniquely owned box-pointer that we must deallocate.
            _ = unsafe { Box::from_raw(fullptr as *mut ManuallyDrop<ItemHolder<T, NoMeta>>) };
        } else {
            let fullptr = get_full_ptr_raw::<T, Metadata<T>>(ptr2);
            // SAFETY:
            // fullptr is a valid pointer
            let layout = get_holder_layout::<T, Metadata<T>>(&unsafe { &*fullptr }.payload);
            // SAFETY:
            // fullptr is a valid uniquely owned pointer that we must deallocate
            unsafe { std::alloc::dealloc(fullptr as *mut _, layout) }
        }
        false
    } else {
        debug_println!("Dropping holder {:?}, including payload", ptr2);
        // SAFETY:
        // `ptr` is always a valid pointer.
        // At this position we've established that we can drop the pointee.
        if is_sized::<T>() {
            let fullptr = get_full_ptr_raw::<T, NoMeta>(ptr2);
            // SAFETY:
            // fullptr is a valid pointer
            _ = unsafe { Box::from_raw(fullptr as *mut ItemHolder<T, NoMeta>) };
        } else {
            let fullptr =
                get_full_ptr_raw::<T, Metadata<T>>(ptr2) as *mut ItemHolder<T, Metadata<T>>;
            // SAFETY:
            // fullptr is a valid pointer
            let layout = get_holder_layout::<T, Metadata<T>>(&unsafe { &*fullptr }.payload);
            // SAFETY:
            // fullptr is a valid pointer
            let payload_ptr = addr_of_mut!(unsafe { &mut *fullptr }.payload);
            // SAFETY:
            // payload_ptr is a pointer to a payload item T that we can drop
            unsafe {
                drop_in_place(payload_ptr);
            }
            // SAFETY:
            // fullptr is a uniquely owned valid pointer that we must deallocate
            unsafe { std::alloc::dealloc(fullptr as *mut _, layout) }
        }
        true
    }
}

/// SAFETY:
/// 'old_ptr' must be a valid ItemHolder-pointer.
fn drop_root_item<T: ?Sized>(old_ptr: *const ItemHolderDummy<T>, strength: usize) {
    debug_println!(
        "drop_root_item about to reduce count {:?} by {}",
        old_ptr,
        strength
    );
    verify_item(old_ptr);
    let count = get_refcount(old_ptr).fetch_sub(strength, atomic::Ordering::SeqCst);
    atomic::fence(Ordering::SeqCst); //Just to make loom work
    debug_println!(
        "Drop-root-item {:?}, count {} -> {}",
        old_ptr,
        count,
        count.wrapping_sub(1)
    );
    assert!(count >= strength && count < 1_000_000_000_000);

    if count == strength {
        debug_println!("Begin drop-root-item for {:?}", old_ptr);
        let next = get_next_and_state(old_ptr).load(atomic::Ordering::SeqCst);
        let strong = drop_payload_and_holder(old_ptr);
        if !undecorate(next).is_null() {
            debug_println!(
                "drop_root_item, recurse {:?}, with state {:?} - recursing into {:?} w str {}",
                old_ptr,
                get_state(next),
                next,
                1
            );
            let next_strength = if strong { MAX_ROOTS } else { 1 };
            drop_root_item(undecorate(next), next_strength);
        }
        debug_println!("drop_root_item, calling drop_payload {:?}", old_ptr);
    }
}
fn drop_item<T: ?Sized>(old_ptr: *const ItemHolderDummy<T>) {
    verify_item(old_ptr);
    ArcShift::drop_impl(old_ptr);
}

#[cfg(test)]
pub mod tests;
