#![no_std]
#![cfg_attr(feature = "nightly", feature(ptr_metadata))]
#![deny(warnings)]
#![forbid(clippy::undocumented_unsafe_blocks)]
#![deny(missing_docs)]

//! # Introduction to ArcShift
//!
//! [`ArcShift`] is a data type similar to [`std::sync::Arc`], except that it allows updating
//! the value pointed to. It can be used as a replacement for [`std::sync::Arc<std::sync::Mutex<T>>`], but with
//! significantly smaller overhead for reads.
//!
//! Writing to ArcShift is significantly more expensive than for [`std::sync::RwLock`], so
//! ArcShift is most suited to use cases where updates are infrequent.
//!
//! See the 'Limitations'-heading further down before using!
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
//!     println!("Value in thread 2: '{}'", arc2.get());
//! });
//!
//! j1.join().unwrap();
//! j2.join().unwrap();
//! # }
//! ```
//!
//! # Implementation
//!
//! When ArcShift values are updated, a linked list of all updates is formed. Whenever
//! an ArcShift-instance is reloaded (using [`ArcShift::reload`], [`ArcShift::get`],
//! that instance advances along the linked list to the last
//! node in the list. When no instance exists pointing at a node in the list, it is dropped.
//! It is thus important to periodically call [`ArcShift::reload`] or [`ArcShift::get`] to avoid retaining unneeded values.
//!
//! # Strong points
//! * Easy to use (similar to Arc)
//! * All functions are lock free (see <https://en.wikipedia.org/wiki/Non-blocking_algorithm> )
//! * For use cases where no modification of values occurs, performance is very good (much
//!   better than RwLock or Mutex).
//! * Modifying values is reasonably fast (think, 50-150 nanoseconds), but much slower than Mutex or
//!   RwLock.
//! * The function [`ArcShift::shared_get`] allows access without any overhead
//!   at compared to regular Arc (benchmarks show identical performance to Arc).
//! * ArcShift does not rely on thread-local variables.
//! * ArcShift is no_std compatible (though 'alloc' is required, since ArcShift is a heap
//!   allocating data structure).
//!
//! # Limitations
//!
//! ArcShift achieves its performance at the expense of the following disadvantages:
//!
//! * When modifying the value, the old version of the value lingers in memory until
//!   the last ArcShift that uses it has updated. Such an update only happens when the ArcShift
//!   is accessed using a unique (`&mut`) access (like [`ArcShift::get`] or [`ArcShift::reload`]).
//!   This can be partially mitigated by using the [`ArcShiftWeak`]-type for long-lived
//!   never-reloaded instances.
//! * Modifying the value is approximately 15x-30x more expensive than modifying an `Arc<Mutex<T>>`.
//!   That said, if you're storing anything significantly more complex than an integer, the overhead
//!   of ArcShift may be insignificant.
//! * When the value is modified, the next subsequent reload is slower than an `Arc<RwLock<T>>`
//!   access.
//! * ArcShift is its own datatype. It is in no way compatible with `Arc<T>`.
//! * At most usize::MAX/8 instances of ArcShift or ArcShiftWeak can be created for each value.
//!   (this is because it uses some bits of its weak refcount to store metadata).
//! * ArcShift instances should ideally be owned (or be mutably accessible). This is because
//!   reloading ArcShift requires mutable access to the ArcShift object itself.
//!
//! The last limitation might seem unacceptable, but for many applications it is not
//! hard to make sure each thread/scope has its own instance of ArcShift pointing to
//! the resource. Cloning ArcShift instances is reasonably fast.
//!
//! # Motivation
//!
//! The primary raison d'être for [`ArcShift`] is to be a version of Arc which allows
//! modifying the stored value, with very little overhead over regular Arc for read heavy
//! loads.
//!
//! The motivating use-case for ArcShift is hot-reloadable assets in computer games.
//! During normal usage, assets do not change. All benchmarks and play experience will
//! be dependent only on this baseline performance. Ideally, we therefore want to have
//! a very small performance penalty for the case when assets are *not* updated, comparable
//! to using regular [`std::sync::Arc`].
//!
//! During game development, artists may update assets, and hot-reload is a very
//! time-saving feature. A performance hit during asset-reload is acceptable though.
//! ArcShift prioritizes base performance, while accepting a penalty when updates are made.
//!
//! ArcShift can, of course, be useful in other domains than computer games.
//!
//! # Performance properties
//!
//! Accessing the value stored in an ArcShift instance only requires a regular memory access,
//! not any form of atomic operation. Checking for new values requires a single
//! atomic operation, of the least expensive kind (Ordering::Relaxed). On x86_64,
//! this is the exact same machine operation as a regular memory access, and also
//! on arm it is not an expensive operation.
//! The cost of such access is much smaller than a mutex access, even an uncontended one.
//! In the case where a reload is actually necessary, there is a significant performance impact
//! (but still typically below 150ns for modern machines (2025)).
//!
//!
//! # Implementation
//!
//! The basic idea of ArcShift is that each ArcShift instance points to a small heap block,
//! that contains the pointee value of type T, three reference counts, and 'prev'/'next'-pointers.
//! The 'next'-pointer starts out as null, but when the value in an ArcShift is updated, the
//! 'next'-pointer is set to point to the updated value.
//!
//! This means that each ArcShift-instance always points at valid value of type T. No locking
//! or synchronization is required to get at this value. This is why ArcShift instances are fast
//! to use. There is the drawback that as long as an ArcShift-instance exists, whatever value
//! it points to must be kept alive. Each time an ArcShift instance is accessed mutably, we have
//! an opportunity to update its pointer to the 'next' value. The operation to update the pointer
//! is called a 'reload'.
//!
//! When the last ArcShift-instance releases a particular value, it will be dropped.
//!
//! ArcShiftWeak-instances also keep pointers to the heap blocks mentioned above, but value T
//! in the block can be dropped while being held by an ArcShiftWeak. This means that an ArcShiftWeak-
//! instance only consumes `std::mem::size_of::<T>()` bytes plus 5 words of memory, when the value
//! it points to has been dropped. When the ArcShiftWeak-instance is reloaded, or dropped, that memory
//! is also released.
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
//! ArcShift uses usize data type for the reference counts. However, it reserves three bits for
//! tracking some metadata. This leaves usize::MAX/4 as the maximum usable reference count. To
//! avoid having to check the refcount twice (once before increasing the count), we set the limit
//! at usize::MAX/8, and check the count after the atomic operation. This has the effect that if more
//! than usize::MAX/8 threads clone the same ArcShift instance concurrently, the unsoundness will occur.
//! However, this is considered acceptable, because this exceeds the currently possible number
//! of concurrent threads by a huge safety margin. Also note that usize::MAX/8 ArcShift instances would
//! take up usize::MAX bytes of memory, which is very much impossible in practice. By leaking
//! ArcShift instances in a tight loop it is still possible to achieve a weak count of usize::MAX/8,
//! in which case ArcShift will panic.
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

#[cfg(any(test,feature = "std"))]
extern crate std;

extern crate alloc;

use alloc::boxed::Box;

//TODO: Figure out which fences aren't needed.

/*
TODO: Remove this section.

Talking points:

Arc<[u8]> is actually 2 words (16 bytes on 64-bit platforms).

This works, because the length can't be changed. But for ArcShift<[u8]>, we'd expect to be able
to change the length! Describe the challenge of sized values!

When decreasing a reference count, if it doesn't go to 0, you know _nothing_ about the count.

Watch out for loom's LOOM_CHECKPOINT_FILE. If you change the code, but keep the file, you
can get crashes.

Talk about the 'helper' concept, how lock-free-ness can be achieved by delegating tasks to other
concurrent threads.

"memory reclamation" problem. Mention:
Mention hazard-pointers (haphazard library) and epoch-based (crossbeam)

The fundamental problem of not being able to look at item at all after decreasing refcount to
something else than 0.

Talk about using least significant bits to store flags.

You may think that if two things happen on adjacent instructions, they'll happen mostly atomically
"in practice". Not true, millions of lines of code can occur between any two instructions, because
of task scheduling. And there's no guarantee hardware effects couldn't produce similar effects.

About the tools:

 * Loom - most powerful at finding errors. Very slow. Slightly error-prone. Often bad error messages.
 * Shuttle - faster. Less magic, in some ways easier to work with. Not as many diagnostics mean less tricky errors
 * Miri - very good error messages. Overall very solid. Doesn't produce as much scheduling jitter as loom.

*/

use core::alloc::Layout;

#[allow(unused)]
use core::cell::UnsafeCell;
use core::fmt::{Debug, Formatter};
use core::marker::PhantomData;
use core::mem::{transmute, ManuallyDrop};
use core::ops::Deref;
use core::panic::UnwindSafe;
use core::ptr::{addr_of_mut, null, null_mut, NonNull};
use core::sync::atomic::Ordering;

use crate::deferred::{DropHandler, IDropHandler, StealingDropHandler};
// About unsafe code in this crate:
// Some private functions contain unsafe code, and place limitations on their
// callers, without these private functions being marked unsafe.
// The rationale is that there are lots of operations that simply aren't unsafe, like
// assigning null to a pointer, that could cause UB in unsafe code in this crate.
// This crate is inherently dependent on all the code in it being correct. Therefore,
// marking more functions unsafe buys us very little.
// Note! The API of this crate is 100% safe and UB should be impossible to trigger by using it.
// All public methods are 100% sound, this argument only concerns private methods.

// All atomic primitives are reexported from a
// local module called 'atomic', so we can easily change between using
// types from 'std' (normal case) and types from shuttle/loom testing libraries.

/// Declarations of atomic ops for using Arcshift in production
#[cfg(all(not(loom), not(feature = "shuttle")))]
mod atomic {
    pub use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering, fence};
    #[cfg(test)]
    pub use std::sync::{Arc,Mutex};
    #[cfg(test)]
    pub use std::thread;
}


mod deferred {
    use crate::{debug_println, get_holder_layout, IMetadata, ItemHolder, SizedMetadata};
    #[cfg(feature = "std")]
    use core::any::Any;
    use core::mem::ManuallyDrop;
    #[cfg(feature="std")]
    use std::panic::{AssertUnwindSafe,resume_unwind};
    use core::ptr::addr_of_mut;
    #[allow(unused)]
    use core::sync::atomic::Ordering;

    pub(crate) trait IDropHandler<T: ?Sized, M: IMetadata> {
        fn do_drop(&mut self, ptr: *mut ItemHolder<T, M>);
        fn do_dealloc(&mut self, item_ptr: *const ItemHolder<T, M>);
        fn report_sole_user(&mut self);
    }

    #[derive(Default)]
    pub(crate) struct DropHandler {
        #[cfg(feature = "std")]
        panic: Option<alloc::boxed::Box<dyn Any + Send + 'static>>,
    }

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
            self.sole_user.then_some(self.stolen?)
        }
    }
    impl<T> IDropHandler<T, SizedMetadata> for StealingDropHandler<T> {
        fn do_drop(&mut self, item_ptr: *mut ItemHolder<T, SizedMetadata>) {
            self.regular.run(|| {
                // SAFETY: item_ptr must be valid, guaranteed by caller.
                let payload = unsafe { addr_of_mut!((*item_ptr).payload).read() };
                debug_println!("payload ref created");
                // SAFETY: item_ptr is now uniquely owned, and the flags of 'next' tell us it has not had
                // its payload dropped yet. We can drop it.
                let payload = ManuallyDrop::into_inner(payload.into_inner());
                self.stolen = Some(payload); //This may drop any previously stolen payload. This is the intended semantics.
                debug_println!("payload stolen");
            });
        }

        fn do_dealloc(&mut self, item_ptr: *const ItemHolder<T, SizedMetadata>) {
            self.regular.do_dealloc(item_ptr);
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

        fn do_dealloc(&mut self, item_ptr: *const ItemHolder<T, M>) {
            // SAFETY:
            // item_ptr is still a valid, exclusively owned pointer
            let layout = get_holder_layout(&unsafe { &*item_ptr }.payload);
            // SAFETY:
            // fullptr is a valid uniquely owned pointer that we must deallocate
            debug_println!("Calling dealloc {:x?}", item_ptr);

            #[cfg(feature = "validate")]
            {
                let item_ref = unsafe { &*(item_ptr as *mut ItemHolder<T, M>) };
                item_ref.magic1.store(0xdeaddead, Ordering::Relaxed);
                item_ref.magic2.store(0xdeaddea2, Ordering::Relaxed);
            }

            // SAFETY:
            // item_ptr is still a valid, exclusively owned pointer
            unsafe { alloc::alloc::dealloc(item_ptr as *mut _, layout) }
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

            #[cfg(feature="std")]
            {
                match std::panic::catch_unwind(AssertUnwindSafe(job)) {
                    Ok(()) => {}
                    Err(err) => {
                        self.panic.get_or_insert(err);
                    }
                }
            }
            #[cfg(not(feature="std"))]
            {
                job();
            }
        }
        pub(crate) fn execute(self) {
            #[cfg(feature = "std")]
            if let Some(panic) = self.panic {
                resume_unwind(panic);
            }
        }
    }
}

/// Declarations for verifying Arcshift using 'shuttle'
#[cfg(feature = "shuttle")]
mod atomic {
    #[allow(unused)]
    pub use shuttle::sync::atomic::{fence, AtomicPtr, AtomicUsize, Ordering};


    pub use shuttle::sync::{Arc,Mutex};
    #[allow(unused)]
    pub use shuttle::thread;

}

/// Declarations for verifying Arcshift using 'loom'
#[cfg(loom)]
mod atomic {
    pub use loom::sync::atomic::{fence, AtomicPtr, AtomicUsize, Ordering};
    #[allow(unused)]
    pub use loom::sync::{Mutex,Arc};
    #[allow(unused)]
    pub use loom::thread;
}

#[doc(hidden)]
#[cfg(all(feature = "debug", not(loom)))]
#[macro_export]
macro_rules! debug_println {
    ($($x:tt)*) => {
        println!("{:?}: {}", crate::atomic::thread::current().id(), format!($($x)*))
    }
}

#[doc(hidden)]
#[cfg(all(feature = "debug", loom))]
#[macro_export]
macro_rules! debug_println {
    ($($x:tt)*) => { println!($($x)*) }
}

#[doc(hidden)]
#[cfg(not(feature = "debug"))]
#[macro_export]
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
/// let mut instance = ArcShift::new("test");
/// println!("Value: {:?}", instance.get());
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
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "ArcShift({:?})", self.shared_get())
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
/// ```
/// # use arcshift::ArcShift;
/// ##[cfg(not(any(loom,feature="shuttle")))]
/// # {
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
unsafe impl<T: Send + ?Sized> Send for ArcShift<T> {}

/// SAFETY:
/// If `T` is `Sync`, `ArcShift<T>` can also be `Sync`
unsafe impl<T: Sync + ?Sized> Sync for ArcShiftWeak<T> {}

/// SAFETY:
/// If `T` is `Send`, `ArcShift<T>` can also be `Send`
unsafe impl<T: Send + ?Sized> Send for ArcShiftWeak<T> {}

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

fn to_dummy<T: ?Sized, M: IMetadata>(ptr: *const ItemHolder<T, M>) -> *const ItemHolderDummy<T> {
    ptr as *mut ItemHolderDummy<T>
}
fn from_dummy<T: ?Sized, M: IMetadata>(ptr: *const ItemHolderDummy<T>) -> *const ItemHolder<T, M> {
    get_full_ptr_raw::<T, M>(ptr)
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

fn get_weak_count(count: usize) -> usize {
    let count = count & ((1 << (usize::BITS - 2)) - 1);
    #[cfg(feature = "validate")]
    assert!(count < MAX_REF_COUNT / 2);
    count
}

/// Node has next, or will soon have next
const WEAK_HAVE_NEXT: usize = 1 << (usize::BITS - 1);
const WEAK_HAVE_PREV: usize = 1 << (usize::BITS - 2);

/// To avoid potential unsafety if refcounts overflow and wrap around,
/// we have a maximum limit for ref count values. This limit is set with a large margin relative
/// to the actual wraparound value. Note that this
/// limit is enforced post fact, meaning that if there are more than usize::MAX/8 or so
/// simultaneous threads, unsoundness can occur. This is deemed acceptable, because it is
/// impossible to achieve in practice. Especially since it basically requires an extremely long
/// running program with memory leaks, or leaking memory very fast in a tight loop. Without leaking,
/// is is impractical to achieve such high refcounts, since having usize::MAX/8 ArcShift instances
/// alive on a 64-bit machine is impossible, since this would require usize::MAX/2 bytes of memory,
/// orders of magnitude larger than any existing machine (in 2025).
const MAX_REF_COUNT: usize = usize::MAX / 8;

fn get_weak_prev(count: usize) -> bool {
    (count & WEAK_HAVE_PREV) != 0
}

fn get_weak_next(count: usize) -> bool {
    (count & WEAK_HAVE_NEXT) != 0
}

fn initial_weak_count<T: ?Sized>(prev: *const ItemHolderDummy<T>) -> usize {
    if prev.is_null() {
        1
    } else {
        1 | WEAK_HAVE_PREV
    }
}

#[allow(unused)]
#[cfg(test)]
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
            // std::alloc::alloc requires the allocated layout to have a nonzero size. This
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

struct SizedMetadata;
trait IMetadata {}
impl IMetadata for SizedMetadata {}
impl<T: ?Sized> IMetadata for UnsizedMetadata<T> {}

#[repr(transparent)]
struct ItemHolderDummy<T: ?Sized> {
    // void pointers should point to u8
    dummy: u8,
    phantom_data: PhantomData<T>,
}

/*
 * ItemHolder cannot be dropped if weak-count > 0
 * ItemHolder cannot be dropped while strong-count > 0
 * The rightmost ItemHolder cannot be dropped if there are other items (to the left)
   (But it may be possible to apply the janitor operation and remove the 'prev')
 * However, an ItemHolder can have its strong-count increased even if it is dropped,
   so just because strong-count is > 0 doesn't mean the item isn't dropped.
 * If strong-count > 0, weak-count is also > 0. Always.
 * The first taken strong-count owns a weak ref on the item. When the last strong count is
   released, the weak ref must be released.
 * ItemHolder cannot be dropped if item.prev.advance_count > 0
 * ItemHolder can only be dropped when holding a lock on item,
   and a lock on item 'item.next', after having set 'next' to 'item.next' (or further to the right).
   See do_advance_impl for details regarding 'advance_count'.
 * While dropping, the janitor process must be run. If a concurrent janitor task is detected,
   it must be marked as 'disturbed', and the disturbed task must re-run from the beginning
   after completing.
*/

/// Align 8 is needed, since we store flags in the lower 2 bits of the ItemHolder-pointers
/// In practice, the alignment of ItemHolder is 8 anyway, but we specify it here for clarity.
#[repr(align(8))]
#[repr(C)] // Just to get the 'magic' first and last in memory. Shouldn't hurt.
struct ItemHolder<T: ?Sized, M: IMetadata> {
    the_meta: M,
    #[cfg(feature = "validate")]
    magic1: std::sync::atomic::AtomicU64,
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
    /// Can be incremented to keep the 'next' value alive. If 'next' is set to a new value, and
    /// 'advance_count' is read as 0 after, then this node is definitely not going to advance to
    /// some node *before* 'next'.
    advance_count: atomic::AtomicUsize,
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
    fn eq(&self, other: &ItemHolder<T, M>) -> bool {
        core::ptr::addr_eq(self as *const _, other as *const _)
    }
}

impl<T: ?Sized, M: IMetadata> ItemHolder<T, M> {
    fn set_next(&self, undecorated_new_next: *const ItemHolderDummy<T>) {
        #[cfg(feature = "validate")]
        assert_eq!(
            get_decoration(undecorated_new_next),
            ItemStateEnum::UndisturbedUndecorated
        );

        // Lock-free because we only loop if 'next' changes, which means
        // some other node has made progress.
        loop {
            atomic::fence(Ordering::SeqCst);
            let prior_next = self.next.load(atomic::Ordering::Relaxed);
            let new_next = decorate(undecorated_new_next, get_decoration(prior_next));
            if self
                .next
                .compare_exchange(
                    prior_next,
                    new_next as *mut _,
                    atomic::Ordering::AcqRel,
                    atomic::Ordering::Acquire,
                )
                .is_ok()
            {
                debug_println!(
                    "set {:x?}.next = {:x?}",
                    to_dummy(self as *const _),
                    new_next
                );
                return;
            }
        }
    }

    /// Includes self
    #[allow(unused)]
    #[cfg(test)]
    fn debug_all_to_left<'a>(&self) -> std::vec::Vec<&'a ItemHolder<T, M>> {
        let mut ret = std::vec![];
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

    #[cfg(feature="debug")]
    fn decoration(&self) -> ItemStateEnum {
        get_decoration(self.next.load(Ordering::SeqCst))
    }

    fn has_payload(&self) -> bool {
        !get_decoration(self.next.load(atomic::Ordering::Acquire)).is_dropped()
    }

    #[inline(always)]
    unsafe fn payload<'a>(&self) -> &'a T {
        // SAFETY:
        // Lifetime extension of shared references like this is permissible, as long as the object,
        // remains alive. Because of our refcounting, the object is kept alive as long as
        // the accessor (ArcShift-type) that calls this remains alive.
        unsafe { transmute::<&T, &'a T>(&*(self.payload.get())) }
    }
    /// Returns true if the node could be atomically locked
    fn lock_node_for_gc(&self) -> bool {
        // Lock free, see comment for each 'continue' statement, which are the only statements
        // that lead to looping.
        loop {
            atomic::fence(atomic::Ordering::SeqCst);
            let cur_next = self.next.load(atomic::Ordering::Relaxed);
            let decoration = get_decoration(cur_next);
            if decoration.is_unlocked() {
                let decorated = decorate(undecorate(cur_next), decoration.with_gc());
                let success = self
                    .next
                    .compare_exchange(
                        cur_next,
                        decorated as *mut _,
                        Ordering::AcqRel,
                        atomic::Ordering::Acquire,
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
                    "Locking node {:x?} failed, already decorated",
                    self as *const ItemHolder<T, M>
                );
                if !decoration.is_disturbed() {
                    let decorated = decorate(undecorate(cur_next), decoration.with_disturbed());
                    match self.next.compare_exchange(
                        cur_next,
                        decorated as *mut _,
                        Ordering::AcqRel,
                        atomic::Ordering::Acquire
                    ) {
                        Ok(_) => {
                            atomic::fence(Ordering::SeqCst);
                            debug_println!("Locking node {:x?} failed, set disturbed flag, new value: {:x?} (={:?})", self as * const ItemHolder<T, M>, decorated, get_decoration(decorated));
                        }
                        Err(prior) => {
                            if !get_decoration(prior).is_disturbed() {
                                // Lock free, because we can only end up here if some other thread did:
                                // 1) Added a new node, which is considered progress
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

            atomic::fence(Ordering::SeqCst);
            let cur_next = self.next.load(atomic::Ordering::Relaxed);
            let decoration = get_decoration(cur_next);
            let undecorated = decorate(undecorate(cur_next), decoration.without_gc_and_disturbed());

            #[cfg(feature = "validate")]
            assert!(
                decoration.is_locked(),
                "node {:x?} was not actually locked",
                self as *const _
            );

            debug_println!("Unlocked dec {:x?}: {:x?}", self as *const Self, decoration);
            if self
                .next
                .compare_exchange(
                    cur_next,
                    undecorated as *mut _,
                    Ordering::AcqRel,
                    atomic::Ordering::Acquire
                )
                .is_ok()
            {
                atomic::fence(Ordering::SeqCst);
                return decoration.is_disturbed();
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
#[cfg(feature = "validate")]
#[cfg_attr(test, mutants::skip)] // This is only used for validation and test, it has no behaviour
impl<T: ?Sized, M: IMetadata> Drop for ItemHolder<T, M> {
    fn drop(&mut self) {
        Self::verify(self as *const _);
        debug_println!("ItemHolder<T>::drop {:?}", self as *const ItemHolder<T, M>);
        {
            self.magic1 = std::sync::atomic::AtomicU64::new(0xDEADDEA1DEADDEA1);
            self.magic2 = std::sync::atomic::AtomicU64::new(0xDEADDEA2DEADDEA2);
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
fn undecorate<T: ?Sized>(cand: *const ItemHolderDummy<T>) -> *const ItemHolderDummy<T> {
    let raw = cand as usize & 7;
    (cand as *const u8).wrapping_offset(-(raw as isize)) as *const ItemHolderDummy<T>
}

impl<T: ?Sized> Clone for ArcShift<T> {
    fn clone(&self) -> Self {
        let t = with_holder!(self.item, T, |item: *const ItemHolder<T, _>| {
            let mut drop_jobs = DropHandler::default();
            let result = do_clone_strong(item, &mut drop_jobs);
            drop_jobs.execute();
            result
        });
        ArcShift {
            // SAFETY:
            // The pointer returned by 'do_clone_strong' is always valid and non-null.
            item: unsafe { NonNull::new_unchecked(t as *mut _) },
        }
    }
}
impl<T: ?Sized> Clone for ArcShiftWeak<T> {
    fn clone(&self) -> Self {
        let t = with_holder!(self.item, T, |item: *const ItemHolder<T, _>| {
            do_clone_weak(item)
        });
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
    atomic::fence(Ordering::SeqCst);
    // SAFETY:
    // do_clone_strong must be called with a valid item_ptr
    let item = unsafe { &*item_ptr };
    debug_println!(
        "Strong clone, about to access strong count of {:x?} (weak: {})",
        item_ptr,
        format_weak(item.weak_count.load(Ordering::Relaxed))
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
    atomic::fence(Ordering::SeqCst);
    // SAFETY:
    // do_clone_weak must be called with a valid item_ptr
    let item = unsafe { &*item_ptr };
    let weak_count = item.weak_count.fetch_add(1, Ordering::Relaxed);
    if get_weak_count(weak_count) > MAX_REF_COUNT {
        item.weak_count.fetch_sub(1, Ordering::Relaxed);
        panic!("weak ref count max limit exceeded")
    }
    debug_println!(
        "weak-clone, fetch_add, new weak count: {:x?} is {}",
        item_ptr,
        format_weak(weak_count + 1)
    );
    #[cfg(feature = "validate")]
    assert!(weak_count > 0);
    to_dummy(item_ptr)
}

fn do_upgrade_weak<T: ?Sized, M: IMetadata>(
    item_ptr: *const ItemHolder<T, M>,
    jobq: &mut DropHandler,
) -> Option<*const ItemHolderDummy<T>> {
    debug_println!("executing do_upgrade_weak");
    // SAFETY:
    // do_upgrade_weak must be called with a valid item_ptr
    let start_item = unsafe { &*(item_ptr) };
    {
        // This is needed, since ostensibly, this method works on a weak clone of 'item_ptr'.
        // It is this weak clone that is converted to a strong item.
        let weak_count = start_item.weak_count.fetch_add(1, Ordering::Relaxed);
        if get_weak_count(weak_count) > MAX_REF_COUNT {
            start_item.weak_count.fetch_sub(1, Ordering::AcqRel);
            panic!("weak ref count max limit exceeded")
        }
        debug_println!(
            "do_upgrade_weak {:x?} incr weak count to {}",
            item_ptr,
            format_weak(weak_count + 1)
        );
    }
    let mut item_ptr = to_dummy(item_ptr);
    // Lock free: See comment on each 'continue'
    loop {

        item_ptr = do_advance_weak::<T, M>(item_ptr);

        // SAFETY:
        // do_advance_weak always returns a valid non-null pointer
        let item = unsafe { &*from_dummy::<T, M>(item_ptr) };
        let prior_strong_count = item.strong_count.load(Ordering::Acquire);
        let item_next = item.next.load(Ordering::Acquire);
        if !get_decoration(item_next).is_dropped() {
            if prior_strong_count == 0 {
                let _prior_weak_count = item.weak_count.fetch_add(1, Ordering::AcqRel);
                debug_println!(
                    "pre-upgrade strong=0 {:x?}, increase weak to {}",
                    item_ptr,
                    get_weak_count(_prior_weak_count) + 1
                );
            }

            if item
                .strong_count
                .compare_exchange(
                    prior_strong_count,
                    prior_strong_count + 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                // SAFETY:
                // item_ptr has been advanced to, and we thus hold a weak ref.
                // it is a valid pointer.
                let item = unsafe { &*from_dummy::<T, M>(item_ptr) };
                let item_next = item.next.load(Ordering::Acquire);
                if get_decoration(item_next).is_dropped() {
                    let new_prior_strong = item.strong_count.fetch_sub(1, Ordering::AcqRel);
                    if new_prior_strong == 1 {
                        let _prior_weak_count = item.weak_count.fetch_sub(1, Ordering::AcqRel);
                        #[cfg(feature = "validate")]
                        assert!(get_weak_count(_prior_weak_count) > 1);
                    }
                    // Lock free. We only get here if some othe rnode has dropped 'item_next'
                    // which is considered progress.
                    continue;
                }

                let _reduce_weak = item.weak_count.fetch_sub(1, Ordering::AcqRel);
                debug_println!(
                    "upgrade success, new strong_count: {}, new weak: {}",
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
                    let _prior_weak_count = item.weak_count.fetch_sub(1, Ordering::AcqRel);
                    debug_println!(
                        "pre-upgrade strong=0 race2 {:x?}, decrease weak to {}",
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
    // However, update only fails if there has been system-wide progress (see comment in
    // do_advance_strong).
    loop {

        atomic::fence(Ordering::SeqCst);
        debug_println!("In advance-loop, item_ptr: {:x?}", item_ptr);
        // SAFETY:
        // item_ptr is a pointer we have advanced to. Forward advance always visits only
        // valid pointers. This is a core algorithm of ArcShift. By the careful atomic operations
        // below, and by knowing how the janitor-task works, we make sure that either 'advance_count'
        // or 'weak_count' keep all visited references alive, such that they cannot be garbage
        // collected.
        let item: &ItemHolder<T, M> = unsafe { &*from_dummy(item_ptr as *mut _) };

        let next_ptr = undecorate(item.next.load(Ordering::Acquire));
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
        atomic::fence(Ordering::SeqCst);
        let _advanced = item.advance_count.fetch_add(1, Ordering::AcqRel);
        debug_println!(
            "advance: Increasing {:x?}.advance_count to {}",
            item_ptr,
            _advanced + 1
        );
        atomic::fence(Ordering::SeqCst);

        // We must reload next, becuase it's only the next that is _now_ set that is actually
        // protected by 'advance_count' above!
        let next_ptr = undecorate(item.next.load(Ordering::Acquire));
        #[cfg(feature = "validate")]
        assert_ne!(next_ptr, item_ptr);
        // SAFETY:
        // next_ptr is guarded by our advance_count ref that we grabbed above.
        let next: &ItemHolder<T, M> = unsafe { &*from_dummy(next_ptr) };
        debug_println!("advance: Increasing next(={:x?}).weak_count", next_ptr);
        let _res = next.weak_count.fetch_add(1, Ordering::AcqRel);
        atomic::fence(Ordering::SeqCst);
        debug_println!(
            "do_advance_impl: increment weak of {:?}, was {}, now {}, now accessing: {:x?}",
            next_ptr,
            format_weak(_res),
            format_weak(_res + 1),
            item_ptr
        );

        let _advanced = item.advance_count.fetch_sub(1, Ordering::AcqRel);
        atomic::fence(Ordering::SeqCst);
        debug_println!(
            "advance: Decreasing {:x?}.advance_count to {}",
            item_ptr,
            _advanced - 1
        );

        if item_ptr != start_ptr {
            debug_println!("do_advance_impl: decrease weak from {:x?}", item_ptr);
            let _res = item.weak_count.fetch_sub(1, Ordering::AcqRel);
            debug_println!(
                "do_advance_impl: decrease weak from {:x?} - decreased to {}",
                item_ptr,
                format_weak(_res - 1)
            );
            #[cfg(feature = "validate")]
            assert!(get_weak_next(_res));
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
            let _a_weak = unsafe { (*a).weak_count.fetch_sub(1, Ordering::AcqRel) };
            // We have a weak ref count on 'b' given to use by `do_advance_impl`, which we're fine with
            debug_println!(
                "weak advance {:x?}, decremented weak count to {}",
                a,
                format_weak(_a_weak.wrapping_sub(1))
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
        let mut b_strong = unsafe { (*b).strong_count.load(Ordering::Acquire) };
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
            // SAFETY:
            // b is a valid pointer. See above.
            if unsafe { !(*b).has_payload() } {
                debug_println!("do_advance_strong:b has no payload");
                // Lock free, we can only get here if another thread has added a new node.
                return false;
            }

            if b_strong == 0 {
                // a strong ref must _always_ imply a weak count, and that weak count must
                // always be decrementable by whoever happens to decrement the count to 0.
                // That might not be us (because we might race with another invocation of this code),
                // that might also increment the count.
                // SAFETY:
                // b is a valid pointer (see above)
                let _prior_weak = unsafe { (*b).weak_count.fetch_add(1, Ordering::AcqRel) };
                debug_println!(
                    "Prior to strong_count imcrement {:x?} from 0, added weak count. Weak now: {}",
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
                    Ordering::AcqRel,
                    atomic::Ordering::Acquire
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
                    let next_ptr = unsafe { (*b).next.load(Ordering::Acquire) };
                    if !undecorate(next_ptr).is_null() {
                        // Race - even though _did_ have payload prior to us grabbing the strong count, it now doesn't.
                        // This can only happen if there's another node to the right, that _does_ have a payload.

                        // SAFETY:
                        // b is a valid pointer. See above.
                        let prior_strong =
                            unsafe { (*b).strong_count.fetch_sub(1, Ordering::AcqRel) };
                        debug_println!("do_advance_strong:rare race, new _next_ appeared when increasing strong count: {:?} (decr strong back to {}) <|||||||||||||||||||||||||||||||||||||||||||||||||||||||>", b, prior_strong -1);
                        #[cfg(feature = "validate")]
                        assert!(
                            prior_strong > 0,
                            "strong count {:x?} must be >0, but was 0",
                            b
                        );

                        //Error: just because b_strong > 0 doesn't mean it has a weak count.

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
                                unsafe { (*b).weak_count.fetch_sub(1, Ordering::AcqRel) };
                            debug_println!("do_advance_strong:rare2 {:x?} reduced strong count back to 0, reduced weak to: {}", b, get_weak_count(_b_weak_count-1));
                            #[cfg(feature = "validate")]
                            assert!(get_weak_count(_b_weak_count) > 1);
                        }
                        // Lock free, we can only get here if another thread has added a new node.
                        return false;
                    }
                    #[cfg(feature = "validate")]
                    assert!(!get_decoration(next_ptr).is_dropped());

                    // SAFETY:
                    // b is a valid pointer. See above.
                    let _b_weak_count = unsafe { (*b).weak_count.fetch_sub(1, Ordering::AcqRel) };
                    debug_println!(
                        "strong advance {:x?}, reducing free b-count to {:?}",
                        b,
                        format_weak(_b_weak_count - 1)
                    );
                    #[cfg(feature = "validate")]
                    assert!(get_weak_count(_b_weak_count) > 1);

                    // SAFETY:
                    // a is a valid pointer. See above.
                    let a_strong = unsafe { (*a).strong_count.fetch_sub(1, Ordering::AcqRel) };
                    #[cfg(feature = "validate")]
                    assert_ne!(a_strong, 0);
                    debug_println!(
                        "a-strong {:x?} decreased {} -> {} (weak of a: {})",
                        a,
                        a_strong,
                        a_strong - 1,
                        format_weak(unsafe { (*a).weak_count.load(Ordering::Acquire) })
                    );
                    if a_strong == 1 {
                        do_drop_payload_if_possible(a, false, drop_job_queue);
                        // Remove the implicit weak granted by the strong
                        // SAFETY:
                        // a is a valid pointer. See above.
                        let _a_weak = unsafe { (*a).weak_count.fetch_sub(1, Ordering::AcqRel) };
                        debug_println!("do_advance_strong:maybe dropping payload of a {:x?} (because strong-count is now 0). Adjusted weak to {}", a, format_weak(_a_weak.wrapping_sub(1)));
                        #[cfg(feature = "validate")]
                        assert!(get_weak_count(_a_weak) > 0);
                    }
                    return true;
                }
                Err(err_value) => {
                    if b_strong == 0 {
                        // SAFETY:
                        // b is a valid pointer. See above.
                        let _prior_weak = unsafe { (*b).weak_count.fetch_sub(1, Ordering::AcqRel) };
                        debug_println!("do_advance_strong: race on strong_count for {:x?} (restore-decr weak to {})", b, get_weak_count(_prior_weak - 1));
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
        unsafe { (*item_ptr).strong_count.load(Ordering::Acquire) },
        0,
        "{:x?} strong count must be 0 when deallocating",
        item_ptr
    );

    #[cfg(feature = "validate")]
    {
        let item_ref = unsafe { &*(item_ptr as *mut ItemHolder<T, M>) };
        item_ref.magic1.store(0xdeaddead, Ordering::Relaxed);
        item_ref.magic2.store(0xdeaddea2, Ordering::Relaxed);
    }

    jobq.do_dealloc(item_ptr);
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
        return (
            false,
            get_strong_status(have_seen_left_strong_refs, false),
        );
    }

    atomic::fence(Ordering::SeqCst);

    debug_println!("Loading prev of {:x?}", start_ptr);
    let mut cur_ptr: *const _ = start.prev.load(Ordering::SeqCst);
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

            let prev_ptr: *const _ = cur.prev.load(Ordering::SeqCst);
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

                let prev_ptr: *const _ = cur.prev.load(Ordering::SeqCst);
                anyrerun |= cur.unlock_node();
                debug_println!("Cleanup sweep going to {:x?}", prev_ptr);
                cur_ptr = prev_ptr;
            }
            anyrerun |= start.unlock_node();
            debug_println!("Janitor failed to grab locks. Rerun: {:?}", anyrerun);
            return (anyrerun,
                    get_strong_status(have_seen_left_strong_refs, false)
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
        debug_println!(
            "Setting next of {:x?} to {:x?} (weak: {} (weak of start: {})",
            cur_ptr,
            start_ptr,
            format_weak(cur.weak_count.load(Ordering::SeqCst)),
            format_weak(start.weak_count.load(Ordering::SeqCst))
        );
        #[cfg(feature = "validate")]
        assert_ne!(cur_ptr, start_ptr);

        if cur.strong_count.load(Ordering::SeqCst) > 0 {
            have_seen_left_strong_refs = true;
        }

        let prev_ptr: *const _ = cur.prev.load(Ordering::SeqCst);
        debug_println!("Janitor loop considering {:x?} -> {:x?}", cur_ptr, prev_ptr);
        if prev_ptr.is_null() {
            debug_println!(
                "Janitor task for {:x?} - found leftmost node: {:x?}",
                start_ptr,
                cur_ptr
            );
            cur_ptr = null_mut();
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
        atomic::fence(Ordering::SeqCst);

        let adv_count = prev.advance_count.load(Ordering::SeqCst);
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
    // The leftmost stop to this janitor cycle. This node must not be operated on.
    // I.e, it's one-beyond-the-end.
    let end_ptr = cur_ptr;

    fn find_non_deleted_predecessor<T: ?Sized, M: IMetadata>(
        mut item_ptr: *const ItemHolderDummy<T>,
        last_valid: *const ItemHolderDummy<T>,
        jobq: &mut impl IDropHandler<T, M>,
    ) -> Option<*const ItemHolderDummy<T>> {
        let mut deleted_count = 0;

        // Lock free, since this loop at most iterates once per node in the chain.
        loop {

            if item_ptr == last_valid || item_ptr.is_null() {
                debug_println!(
                    "Find non-deleted {:x?}, count = {}, item_ptr = {:x?}, last_valid = {:x?}",
                    item_ptr,
                    deleted_count,
                    item_ptr,
                    last_valid
                );
                return (deleted_count > 0).then_some(item_ptr);
            }
            // SAFETY:
            // caller must ensure item_ptr is valid, or null. And it is not null, we checked above.
            let item: &ItemHolder<T, M> = unsafe { &*from_dummy(item_ptr as *mut _) };
            // Item is *known* to have a 'next'!=null here, so
            // we know weak_count == 1 implies it is only referenced by its 'next'
            let item_weak_count = item.weak_count.load(Ordering::SeqCst);
            if get_weak_count(item_weak_count) > 1 {
                debug_println!(
                    "Find non-deleted {:x?}, count = {}, found node with weak count > 1 ({})",
                    item_ptr,
                    deleted_count,
                    format_weak(item_weak_count)
                );
                return (deleted_count > 0).then_some(item_ptr);
            }
            debug_println!("Deallocating node in janitor task: {:x?}, dec: {:?}, weak count: {}, strong count: {}", item_ptr, unsafe{ (*from_dummy::<T,M>(item_ptr)).decoration()}, format_weak(item_weak_count),
                item.strong_count.load(Ordering::SeqCst)
            );

            #[cfg(feature = "validate")]
            {
                let prior_item_weak_count = item.weak_count.fetch_sub(1, Ordering::SeqCst);
                if get_weak_count(prior_item_weak_count) != 1 {
                    #[cfg(feature = "validate")]
                    assert_eq!(
                        get_weak_count(prior_item_weak_count),
                        1,
                        "{:x?} weak_count should still be 1, and we decrement it to 0",
                        item_ptr
                    );
                }
            }

            let prev_item_ptr = item.prev.load(Ordering::SeqCst);
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

    // Lock free, since this loop just iterates through each node in the chain.
    while cur_ptr != end_ptr && !cur_ptr.is_null() {

        let new_predecessor = must_see_before_deletes_can_be_made
            .is_null()
            .then(|| find_non_deleted_predecessor::<T, M>(cur_ptr, end_ptr, jobq))
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
                    .fetch_and(!WEAK_HAVE_PREV, Ordering::SeqCst);
            }
            right
                .prev
                .store(new_predecessor_ptr as *mut _, Ordering::SeqCst);
            atomic::fence(Ordering::SeqCst);

            if new_predecessor_ptr.is_null() {
                debug_println!("new_predecessor is null");
                break;
            }
            debug_println!("found new_predecessor: {:x?}", new_predecessor_ptr);
            if new_predecessor_ptr == end_ptr {
                break;
            }
            right_ptr = new_predecessor_ptr;
            // SAFETY:
            // We only visit nodes we have managed to lock. new_predecessor is such a node
            // and it is not being deleted. It is safe to access.
            let new_predecessor = unsafe { &*from_dummy::<T, M>(new_predecessor_ptr) };
            cur_ptr = new_predecessor.prev.load(Ordering::SeqCst);
            #[cfg(feature = "validate")]
            assert_ne!(
                new_predecessor_ptr as *const _, end_ptr,
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
            cur_ptr = cur.prev.load(Ordering::SeqCst);
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
        atomic::fence(Ordering::SeqCst);
        item_ptr = do_advance_weak::<T, M>(item_ptr);
        let (need_rerun, strong_refs) =
            do_janitor_task(from_dummy::<T, M>(item_ptr), drop_job_queue);
        atomic::fence(Ordering::SeqCst);
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
        let prior_weak = item.weak_count.load(Ordering::SeqCst);

        #[cfg(feature = "validate")]
        assert!(get_weak_count(prior_weak) > 0);
        atomic::fence(Ordering::SeqCst);


        let next_ptr = item.next.load(Ordering::SeqCst);
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

        #[cfg(feature = "debug")]
        if get_weak_next(prior_weak) {
            let _next_ptr = item.next.load(Ordering::SeqCst);
            debug_println!("raced in drop weak - {:x?} was previously advanced to rightmost, but now it has a 'next' again (next is {:?})", item_ptr, _next_ptr);
        }


        debug_println!("No add race, prior_weak: {}", format_weak(prior_weak));

        // We now have enough information to know if we can drop payload, if desired
        {
            // SAFETY:
            // item_ptr was returned by do_advance_weak, and is thus valid.
            let o_item = unsafe { &*from_dummy::<T, M>(item_ptr) };
            let o_strong = o_item.strong_count.load(Ordering::SeqCst);
            atomic::fence(Ordering::SeqCst);
            let original_next = o_item.next.load(Ordering::SeqCst);
            if o_strong == 0 && !get_decoration(original_next).is_dropped() {
                let can_drop_now;
                #[allow(clippy::if_same_then_else)]
                if !undecorate(original_next).is_null() {
                    debug_println!("final drop analysis {:x?}: Can drop this payload, because 'original.next' now exists", item_ptr);
                    can_drop_now = true;
                } else if strong_refs == NodeStrongStatus::NoStrongRefsExist {
                    debug_println!("final drop analysis {:x?}: Can drop this payload, because no strong refs exists anywhere", item_ptr);
                    can_drop_now = true;
                } else {
                    debug_println!("final drop analysis {:x?}: no exemption condition found, can't drop this payload", item_ptr);
                    can_drop_now = false;
                }
                if can_drop_now {
                    do_drop_payload_if_possible(o_item, can_drop_now, drop_job_queue);
                }
            }
        }

        #[cfg(feature = "debug")]
        {
            let _have_prev = !item.prev.load(Ordering::SeqCst).is_null();
            debug_println!("do_drop_weak: reducing weak count of {:x?}, to {} -> {} (have next/gc: {}, have prev: {}) ", item_ptr, format_weak(prior_weak), format_weak(prior_weak.wrapping_sub(1)), false,_have_prev);
        }

        match item.weak_count.compare_exchange(
            prior_weak,
            prior_weak - 1,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => {
                debug_println!(
                    "drop weak {:x?}, did reduce weak to {}",
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

                continue;
            }
        }

        debug_println!(
            "Prior weak of {:x?} is {}",
            item_ptr,
            format_weak(prior_weak)
        );
        if get_weak_count(prior_weak) == 1 {
            if get_weak_next(prior_weak) {
                #[cfg(test)]
                {
                    std::println!("This case should not be possible, since it implies the 'next' object didn't increase the refcount");
                    std::process::abort();
                }
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

        //let new_node = val_dummy;

        item_ptr = do_advance_strong::<T, M>(item_ptr, drop_job_queue);

        // SAFETY:
        // item_ptr was returned by do_advance_strong, and is thus valid.
        let item = unsafe { &*from_dummy::<T, M>(item_ptr) };
        debug_println!(
            "Updating {:x?} , loading next of {:x?}",
            initial_item_ptr,
            item_ptr
        );
        atomic::fence(Ordering::SeqCst);
        let cur_next = item.next.load(Ordering::SeqCst);
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

        atomic::fence(Ordering::SeqCst);
        let _weak_was = loop {
            let weak_count = item.weak_count.load(Ordering::SeqCst);
            let new_weak_count = (weak_count|WEAK_HAVE_NEXT) + 1;
            if item.weak_count.compare_exchange(weak_count, new_weak_count, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                debug_println!("update weak race. {:?} (to {})", item_ptr, format_weak(new_weak_count));
                continue;
            }
            break weak_count;
        };

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
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => {
                atomic::fence(Ordering::SeqCst);
                debug_println!("aft1 item.next.compare_exchange");
            }
            Err(_) => {
                debug_println!("aft2 item.next.compare_exchange");
                debug_println!("race, update of {:x?} to {:x?} failed", item_ptr, new_next);
                let _res = item.weak_count.fetch_sub(1, Ordering::SeqCst);
                debug_println!(
                    "race, decreasing {:x?} weak to {}",
                    item_ptr,
                    format_weak(_res - 1)
                );
                #[cfg(feature = "validate")]
                assert!(get_weak_count(_res) > 1);

                continue;
            }
        }

        //let _prev_weak = format_weak(item.weak_count.load(Ordering::SeqCst));
        let strong_count = item.strong_count.fetch_sub(1, Ordering::SeqCst);
        debug_println!(
            "do_update: strong count {:x?} is now decremented to {}",
            item_ptr,
            strong_count - 1,
        );
        if strong_count == 1 {
            // It's safe to drop payload here, we've just now added a new item
            // that has its payload un-dropped, so there exists something to advance to.
            do_drop_payload_if_possible(from_dummy::<T, M>(item_ptr), false, drop_job_queue);
            let _weak_count = item.weak_count.fetch_sub(1, Ordering::SeqCst);
            debug_println!(
                "do_update: decrement weak of {:?}, new weak: {}",
                item_ptr,
                format_weak(_weak_count.saturating_sub(1))
            );
            #[cfg(feature = "validate")]
            assert!(get_weak_count(_weak_count) > 1);
        }

        item_ptr = val_dummy;
        // Lock free
        // Only loops if another thread has set the 'disturbed' flag, meaning it has offloaded
        // work on us, and it has made progress, which means there is system wide progress.
        loop {

            atomic::fence(Ordering::SeqCst);
            let (need_rerun, _strong_refs) =
                do_janitor_task(from_dummy::<T, M>(item_ptr), drop_job_queue);
            if need_rerun {
                item_ptr = do_advance_strong::<T, M>(item_ptr, drop_job_queue);
                debug_println!("Janitor {:?} was disturbed. Need rerun", item_ptr);
                continue;
            }
            return item_ptr;
        }
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

        atomic::fence(Ordering::SeqCst);
        let next_ptr = item.next.load(Ordering::SeqCst);
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
        atomic::fence(Ordering::SeqCst);
        match item.next.compare_exchange(
            next_ptr,
            decorate(undecorate(next_ptr), decoration.dropped()) as *mut _,
            Ordering::SeqCst,
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
    debug_println!("Dropping payload {:x?} (dec: {:?})", item_ptr, unsafe {
        (*item_ptr).decoration()
    });
    // SAFETY: item_ptr must be valid, guaranteed by caller.

    drop_queue.do_drop(item_ptr as *mut _);
}

// Caller must guarantee poiner is uniquely owned
fn raw_do_unconditional_drop_payload_if_not_dropped<T: ?Sized, M: IMetadata>(
    item_ptr: *mut ItemHolder<T, M>,
    drop_job_queue: &mut impl IDropHandler<T, M>,
) {
    debug_println!("Unconditional drop of payload {:x?}", item_ptr);
    // SAFETY: Caller must guarantee poiner is uniquely owned
    let item = unsafe { &*(item_ptr) };
    let next_ptr = item.next.load(Ordering::SeqCst);
    let decoration = get_decoration(next_ptr);
    if !decoration.is_dropped() {
        debug_println!(
            "Actual drop of payload {:x?}, it wasn't already dropped",
            item_ptr
        );
        drop_job_queue.do_drop(item_ptr);
        atomic::fence(Ordering::SeqCst);
    }
}

fn do_drop_strong<T: ?Sized, M: IMetadata>(
    full_item_ptr: *const ItemHolder<T, M>,
    drop_job_queue: &mut impl IDropHandler<T, M>,
) {
    debug_println!(
        "drop strong of {:x?} (strong count: {:?}, weak: {})",
        full_item_ptr,
        unsafe { (*full_item_ptr).strong_count.load(Ordering::SeqCst) },
        get_weak_count(unsafe { (*full_item_ptr).weak_count.load(Ordering::SeqCst) })
    );
    let mut item_ptr = to_dummy(full_item_ptr);

    item_ptr = do_advance_strong::<T, M>(item_ptr, drop_job_queue);
    // SAFETY: do_advance_strong always returns a valid pointer
    let item: &ItemHolder<T, M> = unsafe { &*from_dummy(item_ptr) };

    let prior_strong = item.strong_count.fetch_sub(1, Ordering::SeqCst);
    debug_println!(
        "drop strong of {:x?}, prev count: {}, now {} (weak is {})",
        item_ptr,
        prior_strong,
        prior_strong - 1,
        format_weak(item.weak_count.load(Ordering::SeqCst))
    );
    if prior_strong == 1 {
        // This is the only case where we actually might need to drop the rightmost node's payload
        // (if all nodes have only weak references)
        do_drop_weak::<T, M>(item_ptr, drop_job_queue);
    }
}

impl<T: ?Sized> Drop for ArcShift<T> {
    fn drop(&mut self) {
        atomic::fence(Ordering::SeqCst);
        verify_item(self.item.as_ptr());
        debug_println!("executing ArcShift::drop({:x?})", self.item.as_ptr());
        with_holder!(self.item, T, |item: *const ItemHolder<T, _>| {
            let mut jobq = DropHandler::default();
            do_drop_strong(item, &mut jobq);
            jobq.execute();
        })
    }
}
impl<T: ?Sized> Drop for ArcShiftWeak<T> {
    fn drop(&mut self) {
        atomic::fence(Ordering::SeqCst);
        verify_item(self.item.as_ptr());

        fn drop_weak_helper<T: ?Sized, M: IMetadata>(item: *const ItemHolder<T, M>) {
            let mut jobq = DropHandler::default();
            do_drop_weak::<T, M>(to_dummy(item), &mut jobq);
            jobq.execute();
        }

        with_holder!(self.item, T, |item: *const ItemHolder<T, _>| {
            drop_weak_helper(item)
        })
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

impl<T: ?Sized> Deref for ArcShift<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        with_holder!(self.item, T, |item: *const ItemHolder<T, _>| -> &T {
            // SAFETY:
            // ArcShift has a strong ref, so payload must not have been dropped.
            unsafe { (*item).payload() }
        })
    }
}

impl<T: ?Sized> ArcShiftWeak<T> {
    /// Upgrade this ArcShiftWeak instance to a full ArcShift.
    /// This is required to be able to access any stored value.
    /// If the ArcShift instance has no value (because the last ArcShift instance
    /// had been deallocated), this method returns None.
    pub fn upgrade(&self) -> Option<ArcShift<T>> {
        let t = with_holder!(self.item, T, |item: *const ItemHolder<T, _>| {
            let mut drop_handler = DropHandler::default();
            let temp = do_upgrade_weak(item, &mut drop_handler);
            drop_handler.execute();
            temp
        });
        Some(ArcShift {
            // SAFETY:
            // do_upgrade_weak returns a valid upgraded pointer
            item: unsafe { NonNull::new_unchecked(t? as *mut _) },
        })
    }
}
impl<T> ArcShift<T> {
    /// Drops this ArcShift instance. If this was the last instance of the entire chain,
    /// the payload value is returned.
    pub fn try_into_inner(self) -> Option<T> {
        let mut drop_handler = StealingDropHandler::default();
        verify_item(self.item.as_ptr());
        debug_println!("executing ArcShift::into_inner({:x?})", self.item.as_ptr());
        do_drop_strong(
            from_dummy::<T, SizedMetadata>(self.item.as_ptr()),
            &mut drop_handler,
        );
        core::mem::forget(self);
        drop_handler.take_stolen()
    }

    /// Crate a new ArcShift instance with the given value.
    pub fn new(val: T) -> ArcShift<T> {
        let holder = make_sized_holder(val, null_mut());
        atomic::fence(Ordering::SeqCst);
        ArcShift {
            // SAFETY:
            // The newly created holder-pointer is valid and non-null
            item: unsafe { NonNull::new_unchecked(to_dummy(holder) as *mut _) },
        }
    }

    /// Update the value of this ArcShift instance (and all derived from it) to the given value.
    ///
    /// Note, other ArcShift instances will not see the new value until they call the
    /// [`ArcShift::get`] or [`ArcShift::reload`] methods.
    pub fn update(&mut self, val: T) {
        let holder = make_sized_holder(val, self.item.as_ptr() as *const ItemHolderDummy<T>);

        with_holder!(self.item, T, |item: *const ItemHolder<T, _>| {
            let mut jobq = DropHandler::default();
            // SAFETY:
            // * do_update returns non-null values.
            // * 'holder' is in fact a thin pointer, just what we need for T, since T is Sized.
            let new_item = unsafe {
                NonNull::new_unchecked(
                    do_update(item, |_| Some(holder as *const _), &mut jobq) as *mut _
                )
            };
            jobq.execute();
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
        (*self).deref()
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
                        raw_do_unconditional_drop_payload_if_not_dropped(
                            from_dummy::<T, SizedMetadata>(holder)
                                as *mut ItemHolder<T, SizedMetadata>,
                            &mut jobq,
                        );
                        jobq.execute();
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
        jobq.execute();
        self.item = new_unique_ptr;
        !cancelled
    }
}

#[doc(hidden)]
#[deprecated = "ArcShiftLight has been removed. Please consider using ArcShiftWeak. It is similar, though not a direct replacement."]
pub struct ArcShiftLight {}

impl<T: ?Sized> ArcShift<T> {
    #[doc(hidden)]
    #[deprecated = "rcu_project was completely removed in ArcShift 0.2. Please use method 'rcu' instead, and just manually access the desired field."]
    pub fn rcu_project(&mut self, _marker: NoLongerAvailableMarker) {
        unreachable!("method cannot be called")
    }
    #[doc(hidden)]
    #[deprecated = "rcu_maybe2 was completely removed in ArcShift 0.2. Please use 'rcu_maybe' instead. It has the same features."]
    pub fn rcu_maybe2(&mut self, _marker: NoLongerAvailableMarker) {
        unreachable!("method cannot be called")
    }

    #[doc(hidden)]
    #[deprecated = "update_shared_box was completely removed in ArcShift 0.2. Shared references can no longer be updated. Please file an issue if this is a blocker!"]
    pub fn update_shared_box(&mut self, _marker: NoLongerAvailableMarker) {
        unreachable!("method cannot be called")
    }
    #[doc(hidden)]
    #[deprecated = "update_shared was completely removed in ArcShift 0.2. Shared references can no longer be updated. Please file an issue if this is a blocker!"]
    pub fn update_shared(&mut self, _marker: NoLongerAvailableMarker) {
        unreachable!("method cannot be called")
    }
    #[doc(hidden)]
    #[deprecated = "make_light was completely removed in ArcShift 0.2. Please use ArcShift::downgrade(&item) instead."]
    pub fn make_light(&mut self, _marker: NoLongerAvailableMarker) {
        unreachable!("method cannot be called")
    }
    #[doc(hidden)]
    #[deprecated = "shared_non_reloading_get now does the same thing as 'shared_get', please use the latter instead."]
    pub fn shared_non_reloading_get(&self) -> &T {
        self.shared_get()
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
        }
    }

    /// Returns a mutable reference to the inner value.
    ///
    /// This method returns None if there are any other ArcShift or ArcShiftWeak instances
    /// pointing to the same object chain.
    pub fn try_get_mut(&mut self) -> Option<&mut T> {
        self.reload();
        with_holder!(self.item, T, |item_ptr: *const ItemHolder<T, _>| {
            // SAFETY:
            // self.item is always a valid pointer for shared access
            let item = unsafe { &*item_ptr };
            let weak_count = item.weak_count.load(Ordering::SeqCst);
            if get_weak_count(weak_count) == 1
                && !get_weak_next(weak_count)
                && !get_weak_prev(weak_count)
                && item.strong_count.load(Ordering::SeqCst) == 1
            {
                let weak_count = item.weak_count.load(Ordering::SeqCst);
                if get_weak_count(weak_count) == 1 && !get_weak_next(weak_count) {
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
                    // Cases 1 and 2 are benign. We guard against case 2 by loading 'weak_count'
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

        with_holder!(self.item, T, |item: *const ItemHolder<T, _>| {
            let mut jobq = DropHandler::default();
            // SAFETY:
            // do_update returns a valid non-null pointer
            let new_item = unsafe {
                NonNull::new_unchecked(do_update(item, |_| Some(holder), &mut jobq) as *mut _)
            };
            jobq.execute();
            self.item = new_item;
        });
    }

    /// Reload this ArcShift instance, and return the latest value.
    #[inline(always)]
    pub fn get(&mut self) -> &T {
        self.reload();
        &*self
    }

    // WARNING! This does not reload the pointer. You will see stale values.
    /// Get the current value of this ArcShift instance.
    ///
    /// This method always returns the previously returned value, even if there
    /// are newer values available. Use [`ArcShift::get`] to get the newest value.
    ///
    /// This method has the advantage that it doesn't require `&mut self` access.
    #[inline(always)]
    pub fn shared_get(&self) -> &T {
        self.deref()
    }

    /// Create an [`ArcShiftWeak<T>`] instance.
    ///
    /// The weak pointer allows the value to be deallocated, if all strong references disappear.
    ///
    /// The returned pointer can later be upgraded back to a regular ArcShift instance, if
    /// there is at least one other strong reference (i.e, [`ArcShift<T>`] instance).
    #[must_use = "this returns a new `ArcShiftWeak` pointer, \
                  without modifying the original `ArcShift`"]
    pub fn downgrade(this: &ArcShift<T>) -> ArcShiftWeak<T> {
        let t = with_holder!(this.item, T, |item: *const ItemHolder<T, _>| {
            do_clone_weak(item)
        });
        ArcShiftWeak {
            // SAFETY:
            // do_clone_weak returns a valid, non-null pointer
            item: unsafe { NonNull::new_unchecked(t as *mut _) },
        }
    }
    #[allow(unused)]
    pub(crate) fn weak_count(&self) -> usize {
        with_holder!(self.item, T, |item: *const ItemHolder<T, _>| -> usize {
            // SAFETY:
            // self.item is a valid pointer
            unsafe { get_weak_count((*item).weak_count.load(Ordering::SeqCst)) }
        })
    }
    #[allow(unused)]
    pub(crate) fn strong_count(&self) -> usize {
        with_holder!(self.item, T, |item: *const ItemHolder<T, _>| -> usize {
            // SAFETY:
            // self.item is a valid pointer
            unsafe { (*item).strong_count.load(Ordering::SeqCst) }
        })
    }

    /// Update this instance to the latest value.
    #[inline(always)]
    pub fn reload(&mut self) {
        #[inline(always)]
        fn advance_strong_helper<T: ?Sized, M: IMetadata>(
            item_ptr: *const ItemHolder<T, M>,
        ) -> *const ItemHolderDummy<T> {
            // SAFETY:
            // self.item (ptr) is a valid non-null pointer.
            // Doing a relaxed load here is safe. It doesn't establish a happens-after relationship
            // with any prior stores, but we deem this acceptable for the performance benefit.
            // See crate documentation.
            if unsafe { undecorate((*item_ptr).next.load(Ordering::Relaxed)).is_null() } {
                // Nothing to upgrade to
                // This is a fast-path optimization
                return core::ptr::null();
            }
            let mut jobq = DropHandler::default();

            let mut item_ptr = to_dummy(item_ptr);
            // Lock free. Only loops if disturb-flag was set, meaning other thread
            // has offloaded work on us, and it has made progress.
            loop {

                item_ptr = do_advance_strong::<T, M>(item_ptr, &mut jobq);
                atomic::fence(Ordering::SeqCst);
                let (need_rerun, _strong_refs) =
                    do_janitor_task(from_dummy::<T, M>(item_ptr), &mut jobq);
                if need_rerun {
                    debug_println!("Janitor {:?} was disturbed. Need rerun", item_ptr);
                    continue;
                }
                jobq.execute();
                return item_ptr;
            }
        }
        let advanced = with_holder!(self.item, T, |item: *const ItemHolder<T, _>| {
            advance_strong_helper(item)
        });
        if advanced.is_null() {
            return;
        }

        // SAFETY:
        // 'advanced' is either null, or a value returned by 'do_advance_strong'.
        // it isn't null, since we checked that above. And 'do_advance_strong' always
        // returns valid non-null pointers.
        self.item = unsafe { NonNull::new_unchecked(advanced as *mut _) };
    }

    /// Check all links and refcounts.
    ///
    /// # SAFETY
    /// This method requires that no other threads access the chain while it is running.
    /// It is up to the caller to actually ensure this.
    #[allow(unused)]
    #[cfg(test)]
    pub(crate) unsafe fn debug_validate(
        strong_handles: &[&Self],
        weak_handles: &[&ArcShiftWeak<T>],
    ) {
        let first = if !strong_handles.is_empty() {
            &strong_handles[0].item
        } else {
            &weak_handles[0].item
        };
        with_holder!(first, T, |item: *const ItemHolder<T, _>| {
            Self::debug_validate_impl(strong_handles, weak_handles, item);
        });
    }
    #[allow(unused)]
    #[cfg(test)]
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
            atomic::fence(Ordering::SeqCst);
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

        let mut true_weak_refs = std::collections::HashMap::<*const ItemHolderDummy<T>, usize>::new();
        let mut true_strong_refs = std::collections::HashMap::<*const ItemHolderDummy<T>, usize>::new();
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
            atomic::fence(Ordering::SeqCst);
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

#[cfg(all(not(loom), not(feature = "shuttle")))]
#[cfg(test)]
mod simple_tests {
    use crate::{ArcShift, SizedMetadata};
    use std::thread;
    use alloc::boxed::Box;
    use std::string::ToString;

    #[test]
    fn simple_create() {
        let x = ArcShift::new(Box::new(45u32));
        assert_eq!(**x, 45);
        // SAFETY:
        // No threading involved, &x is valid.
        unsafe { ArcShift::debug_validate(&[&x], &[]) };
    }

    #[test]
    fn simple_janitor_test() {
        let mut x = ArcShift::new(45u32);
        x.update(46);
        // SAFETY:
        // No threading involved, item pointer of ArcShift is always valid
        let item = unsafe { &*crate::from_dummy::<u32,SizedMetadata>(x.item.as_ptr()) };
        let prev = item.prev.load(crate::atomic::Ordering::SeqCst);
        assert_eq!(prev, core::ptr::null_mut());
        assert_eq!(*x, 46);
    }


    #[test]
    fn simple_create_and_update_once() {
        let mut x = ArcShift::new(Box::new(45u32));
        assert_eq!(**x, 45);
        x.update(Box::new(1u32));
        assert_eq!(**x, 1);
        // SAFETY:
        // No threading involved, &x is valid.
        unsafe { ArcShift::debug_validate(&[&x], &[]) };
    }
    #[test]
    fn simple_create_and_update_twice() {
        let mut x = ArcShift::new(Box::new(45u32));
        assert_eq!(**x, 45);
        x.update(Box::new(1u32));
        assert_eq!(**x, 1);
        x.update(Box::new(21));
        assert_eq!(**x, 21);
        // SAFETY:
        // No threading involved, &x is valid.
        unsafe { ArcShift::debug_validate(&[&x], &[]) };
    }
    #[test]
    fn simple_create_and_clone() {
        let x = ArcShift::new(Box::new(45u32));
        let y = x.clone();
        assert_eq!(**x, 45);
        assert_eq!(**y, 45);
        // SAFETY:
        // No threading involved, &x and &y are valid.
        unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
    }
    #[test]
    fn simple_create_and_clone_drop_other_order() {
        let x = ArcShift::new(Box::new(45u32));
        let y = x.clone();
        assert_eq!(**x, 45);
        assert_eq!(**y, 45);
        // SAFETY:
        // No threading involved, &x and &y are valid.
        unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
        drop(x);
        drop(y);
    }
    #[test]
    fn simple_downgrade() {
        let x = ArcShift::new(Box::new(45u32));
        let _y = ArcShift::downgrade(&x);
    }

    #[test]
    #[should_panic(expected = "panic: A")]
    fn panic_boxed_drop() {
        struct PanicOnDrop(char);
        impl Drop for PanicOnDrop {
            fn drop(&mut self) {
                panic!("panic: {}", self.0)
            }
        }
        _ = Box::new(PanicOnDrop('A'));
    }
    #[test]
    #[should_panic(expected = "panic: A")]
    fn simple_panic() {
        struct PanicOnDrop(char);
        impl Drop for PanicOnDrop {
            fn drop(&mut self) {
                panic!("panic: {}", self.0)
            }
        }
        // Use a box so that T has a heap-allocation, so miri will tell us
        // if it's dropped correctly (it should be)
        let a = ArcShift::new(Box::new(PanicOnDrop('A')));
        let mut b = a.clone();
        b.update(Box::new(PanicOnDrop('B')));
        drop(b); //This doesn't drop anything, since 'b' is kept alive by next-ref of a
        drop(a); //This will panic, but shouldn't leak memory
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
        // SAFETY:
        // No threading involved, &left and &right are valid.
        unsafe { ArcShift::debug_validate(&[&left, &right], &[]) };
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

        // SAFETY:
        // No threading involved, &x and &y are valid.
        unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
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
        // SAFETY:
        // No threading involved, &x and &y are valid.
        unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
        y.reload();
        assert_eq!(**y, 21);
        // SAFETY:
        // No threading involved, &x and &y are valid.
        unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
    }

    #[test]
    fn simple_create_clone_twice_and_update_twice() {
        let mut x = ArcShift::new(Box::new(45u32));
        let mut y = x.clone();
        // SAFETY:
        // No threading involved, &x and &y are valid.
        unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
        assert_eq!(**x, 45);
        x.update(Box::new(1u32));
        // SAFETY:
        // No threading involved, &x and &y are valid.
        unsafe { ArcShift::debug_validate(&[&x, &y], &[]) };
        let z = x.clone();
        assert_eq!(**x, 1);
        x.update(Box::new(21));
        // SAFETY:
        // No threading involved, &x, &y and &z are valid.
        unsafe { ArcShift::debug_validate(&[&x, &y, &z], &[]) };
        assert_eq!(**x, 21);
        y.reload();
        assert_eq!(**y, 21);
        // SAFETY:
        // No threading involved, &x, &y and &z are valid.
        unsafe { ArcShift::debug_validate(&[&x, &y, &z], &[]) };
    }

    #[test]
    fn simple_threaded() {


        let mut arc = ArcShift::new("Hello".to_string());
        let arc2 = arc.clone();

        let j1 = thread::Builder::new()
            .name("thread1".to_string())
            .spawn(move || {
                //println!("Value in thread 1: '{}'", *arc); //Prints 'Hello'
                arc.update("New value".to_string());
                //println!("Updated value in thread 1: '{}'", *arc); //Prints 'New value'
            })
            .unwrap();

        let j2 = thread::Builder::new()
            .name("thread2".to_string())
            .spawn(move || {
                // Prints either 'Hello' or 'New value', depending on scheduling:
                let _a = arc2;
                //println!("Value in thread 2: '{}'", arc2.get());
            })
            .unwrap();

        j1.join().unwrap();
        j2.join().unwrap();
    }

    #[test]
    fn simple_upgrade_reg() {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let shift = ArcShift::new(crate::tests::leak_detection::InstanceSpy::new(
            count.clone(),
        ));
        // SAFETY:
        // No threading involved, &shift is valid.
        unsafe { ArcShift::debug_validate(&[&shift], &[]) };
        let shiftlight = ArcShift::downgrade(&shift);
        // SAFETY:
        // No threading involved, &shift and &shiftlight is valid.
        unsafe { ArcShift::debug_validate(&[&shift], &[&shiftlight]) };

        debug_println!("==== running shift.get() = ");
        let mut shift2 = shiftlight.upgrade().unwrap();
        debug_println!("==== running arc.update() = ");
        // SAFETY:
        // No threading involved, &shift, 2shift2 and &shiftlight are valid.
        unsafe { ArcShift::debug_validate(&[&shift, &shift2], &[&shiftlight]) };
        shift2.update(crate::tests::leak_detection::InstanceSpy::new(
            count.clone(),
        ));

        // SAFETY:
        // No threading involved, &shift, &shift2 and &shiftlight are valid.
        unsafe { ArcShift::debug_validate(&[&shift, &shift2], &[&shiftlight]) };
    }
}

// Module for tests
#[cfg(test)]
mod tests;
