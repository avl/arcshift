//#![deny(warnings)]
//#![forbid(clippy::undocumented_unsafe_blocks)]
//#![deny(missing_docs)]

//! # Introduction to ArcShift
//!
//! ArcShift is a data type similar to Arc, except that it allows updating
//! the value pointed to.
//!
//! ## Example
//! ```rust
//! # if cfg!(loom)
//! # {
//! use std::thread;
//! use arcshift::ArcShift;
//!
//! let mut arc = ArcShift::new("Hello".to_string());
//! let mut arc2 = arc.clone();
//!
//!
//! let j1 = thread::spawn(move||{
//!     println!("Value in thread 1: '{}'", *arc2); //Prints 'Hello'
//!     arc2.update("New value".to_string());
//!     println!("Updated value in thread 1: '{}'", *arc2); //Prints 'New value'
//! });
//!
//! let j2 = thread::spawn(move||{
//!     println!("Value in thread 2: '{}'", *arc); //Prints either 'Hello' or 'New value', depends on scheduling
//! });
//!
//! j1.join().unwrap();
//! j2.join().unwrap();
//! # }
//! ```
//! # Motivation
//!
//! The primary raison d'être for ArcShift is to be a version of Arc which allows
//! modifying the stored value, with very little overhead over regular Arc for cloning,
//! dropping and accessing.
//!
//! That said, for most use cases, the more mature 'arc_swap' crate is probably
//! preferable.
//!
//! The motivating use-case for ArcShift is reloadable assets in computer games.
//! During normal usage, assets do not change. All benchmarks and play experience will
//! be dependent only on this baseline performance. Ideally, we therefore want to have
//! a very small performance penalty for the case when assets are *not* updated.
//!
//! During game development, artists may update assets, and hot-reload is a very
//! time-saving feature. However, a performance hit during asset-reload is acceptable.
//! ArcShift prioritizes base performance, while accepting a penalty when updates are made.
//! ArcShift can, under some circumstances described below, have a lingering (small) performance hit
//! until 'force_update' is called. See documentation for the different functions.
//!
//! # Properties
//!
//! Accessing the value stored in an ArcShift instance only requires a single
//! atomic operation, of the least intrusive kind (Ordering::Relaxed). On x86_64,
//! this is the exact same machine operation as a regular memory access, and also
//! on arm it is not an expensive operation.
//! The cost of such access is much smaller than a mutex access, even an uncontended one.
//!
//! # Strong points
//! * Easy to use (similar to Arc)
//! * All functions are Lock free
//! * For use cases where no modification of values occurs, performance is very good.
//! * Modifying values is reasonably fast (think, 10-50 nanoseconds + cost of memory access).
//! * The function 'shared_non_reloading_get' allows access almost without any overhead at all.
//!   (Only overhead is slightly worse cache performance, because of the reference counters.)
//!
//! # Trade-offs - Limitations
//!
//! ArcShift achieves its performance at the expense of the following disadvantages:
//! * When modifying the value, the old version of the value lingers in memory until
//!   the last ArcShift has been updated. Such an update only happens when the ArcShift
//!   is accessed using an owned (or &mut) access (like 'get' or 'force_reload'). This can
//!   be avoided by using the ArcShiftLight-type for long-lived never-reloaded instances.
//! * Modifying the value is approximately 10x more expensive than modifying `RwLock<Arc<T>>`
//! * When the value is modified, the next subsequent access is slower than an `RwLock<Arc<T>>`
//! * ArcShift is its own datatype. It is no way compatible with `Arc<T>`.
//! * At most 524287 instances of ArcShiftRoot can be created for each value.
//! * At most 35000000000000 instances of ArcShift can be created for each value.
//! * ArcShift instances should ideally be owned (or be mutably accessible) to dereference.
//!
//! The last limitation might seem unacceptable, but for many applications it is not
//! hard to make sure each thread/scope has its own instance of ArcShift. Remember that
//! cloning ArcShift is reasonably fast (basically the cost of a single Relaxed atomic increment).
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
//! instances always consume std::mem::size_of::<T>() bytes of memory, even when the value they
//! point to has been dropped.
//!
//!
//! # Pitfall #1 - lingering memory usage
//!
//! Be aware that ArcShift instances that are just "lying around" without ever being updated,
//! will keep old values around, taking up memory. This is a fundamental drawback of the approach
//! taken by ArcShift. One workaround is to replace long-lived non-reloaded instances of
//! ArcShift with ArcShiftLight. This alleviates the problem.
//!
//! You may also prefer to use the 'ArcSwap' crate (by a different author).
//! It does not have this limitation, as long as its 'Cache' type is not used.
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
//! Since each ArcShift instance takes 8 bytes of space, it takes at least 280TB of memory to even
//! be able to hit this limit. If the limit is somehow reached, there will be a best effort
//! attempt at aborting the program. This is similar to how the rust std library handles overflow
//! of the reference counter on std::sync::Arc. Just as with std::core::Arc, the overflow
//! will be detected in practice. For ArcShift, the overflow will be detected as long as the machine
//! has a even remotely fair scheduler, and less than 100 billion threads (though the conditions for
//! non-detection of std::core::Arc-overflow are even more far fetched).
//!
//! # Comparison to ArcSwap
//! ArcSwap ('arc_swap') is a different crate (by a different author).
//! ArcSwap is probably preferable in most situations. It is more mature, and probably faster
//! in many use cases. ArcSwap does not rely on having mutable access to its instances.
//! If updates do occur, and mutable accesses to ArcShift cannot be provided, ArcSwap is likely
//! going to be much faster because of its ingenious use of thread_locals (and other tricks).
//! Only in the case where data is modified extremely rarely (allowing the use of
//! 'shared_get') or where mutable ArcShift instances can be used (allowing the very fast
//! non-shared &mut self 'get' function), will ArcShift be faster than ArcSwap.
//!
//!
//! # A larger example
//!
//! ```rust
//! use arcshift::ArcShift;
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
//!         // Optional step, making sure to reload ArcShift instances so
//!         // old versions do not linger in RAM.
//!         model.reload();
//!     }
//! }
//! ```



use std::alloc::{Layout};
use std::backtrace::Backtrace;
pub use std::collections::HashSet;
use std::mem::MaybeUninit;
use std::ops::Deref;
use std::panic::UnwindSafe;
use std::process::abort;
use std::ptr::{addr_of_mut, addr_of, null_mut};
use std::sync::atomic::{Ordering};
use crate::ItemStateEnum::Superseded;

#[cfg(not(loom))]
mod atomic {
    pub use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
    pub use std::hint::spin_loop;
    #[allow(unused)]
    pub use std::thread;
}

#[cfg(loom)]
mod atomic {
    pub use loom::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
    #[allow(unused)]
    pub use std::sync::atomic::AtomicU64;
    pub use loom::thread;
    pub use loom::hint::spin_loop;

}

#[cfg(debug_assertions)]
macro_rules! debug_println {
    ($($x:tt)*) => { println!($($x)*) }
}

const MAX_ROOTS: usize = 524288;
const MAX_ARCSHIFT: usize = 35000000000000;

#[cfg(not(debug_assertions))]
macro_rules! debug_println {
    ($($x:tt)*) => ({})
}

/// Smart pointer with similar use case as std::sync::Arc, but with
/// the added ability to atomically replace the contents of the Arc.
/// See crate documentation for more information.
pub struct ArcShift<T:'static> {
    item: *const ItemHolder<T>,
}

impl<T> UnwindSafe for ArcShift<T> {}


/// ArcShiftLight is like ArcShift, except it does not provide overhead-free access.
/// This means that it does not prevent old versions of the payload type from being
/// freed.
///
/// WARNING! Because of implementation reasons, each instance of ArcShiftLight will claim
/// a little bit more memory than size_of::<T>, even if the value inside it has been moved out,
/// and even if all other instances of ArcShift/ArcShiftLight have been dropped.
/// If this limitation is unacceptable, consider using ArcShiftLight<Box<T>> as your datatype,
/// or possibly using a different crate.
pub struct ArcShiftLight<T:'static> {
    item: *const ItemHolder<T>,
}

/// SAFETY:
/// ArcShiftLight can be Send as long as T is Send.
/// ArcShiftLight's mechanisms are compatible with both Send and Sync
unsafe impl<T:'static> Send for ArcShiftLight<T> where T: Send {}

/// SAFETY:
/// ArcShiftLight can be Sync as long as T is Sync.
/// ArcShiftLight's mechanisms are compatible with both Send and Sync
unsafe impl<T:'static> Sync for ArcShiftLight<T> where T: Sync {}

impl<T:'static> Clone for ArcShiftLight<T> {
    fn clone(&self) -> Self {
        // SAFETY:
        // self.item is always a valid pointer
        let mut curitem = self.item;
        loop {
            // SAFETY:
            // curitem is always a pointer reachable through the next-chain beginning at
            // self.item, and is thus always a valid pointer.
            let item = unsafe { &*curitem };
            let next = item.next_and_state.load(Ordering::Acquire);
            if !next.is_null() {
                // SAFETY:
                // If next is not null, it is always a valid pointer
                let nextref = unsafe {&*undecorate(next)};
                curitem = nextref;
                atomic::spin_loop();
                continue;
            }
            let count = item.refcount.load(Ordering::Acquire);
            let rootcount = count&(MAX_ROOTS-1);
            debug_println!("ArcShiftLight {:?} clone count: {} (rootcount: {})", item as *const ItemHolder<T>, count, rootcount);
            assert_ne!(count, 0);
            if rootcount >= MAX_ROOTS - 1 {
                panic!("Max limit of ArcShiftLight clones ({}) was reached", MAX_ROOTS);
            }
            match item.refcount.compare_exchange(count, count + 1, Ordering::Release, Ordering::Relaxed) {
                Ok(_) => {
                    debug_println!("ArcShiftLight clone count successfully updated to: {}", count + 1);
                    break;
                }
                Err(othercount) => {

                    if (othercount&(MAX_ROOTS-1)) >= MAX_ROOTS - 1 {
                        panic!("Max limit of ArcShiftLight clones ({}) was reached", MAX_ROOTS);
                    }
                    atomic::spin_loop();
                    continue;
                }
            }
        }
        debug_println!("Returning new ArcShiftLight for {:?}", curitem);
        ArcShiftLight {
            item: curitem
        }
    }
}
impl<T:'static> ArcShiftLight<T> {
    /// Create a new ArcShift, containing the given type.
    pub fn new(payload: T) -> ArcShiftLight<T> {
        let item = ItemHolder {
            magic1: std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111),
            magic2: std::sync::atomic::AtomicU64::new(0x1234123412348111),
            payload,
            next_and_state: atomic::AtomicPtr::default(),
            refcount: atomic::AtomicUsize::new(1),

        };
        let cur_ptr = Box::into_raw(Box::new(item));
        debug_println!("Created ArcShiftLight for {:?}",  cur_ptr);
        ArcShiftLight {
            item: cur_ptr,
        }
    }

    /// Create an ArcShift instance from this ArcShiftLight.
    pub fn get(&self) -> ArcShift<T> {
        debug_println!("ArcShiftRoot Promoting to ArcShift {:?}", self.item);
        // SAFETY:
        // self.item is always a valid pointer
        let mut curitem = self.item;
        loop {
            // SAFETY:
            // curitem is a pointer reachable through the next-chain beginning at self.item,
            // and is thus always a valid pointer.
            let item = unsafe { &*curitem };
            let next = item.next_and_state.load(Ordering::Acquire);
            if !next.is_null() {
                curitem = undecorate(next);
                atomic::spin_loop();
                continue;
            }
            let _precount = item.refcount.fetch_add(MAX_ROOTS, Ordering::Acquire);
            assert!(_precount >= 1);
            debug_println!("Promote {:?}, prev count: {}, new count {}", curitem, _precount, _precount + MAX_ROOTS);
            let next = item.next_and_state.load(Ordering::Acquire);
            if !next.is_null() {
                let _precount = item.refcount.fetch_sub(MAX_ROOTS, Ordering::Release);
                assert!(_precount > MAX_ROOTS &&  _precount < 1_000_000_000_000);
                curitem = undecorate(next);
                atomic::spin_loop();
                continue;
            }

            let mut temp = ArcShift {
                item: curitem
            };
            temp.reload();
            return temp;
        }
    }
}

impl<T:'static> Drop for ArcShiftLight<T> {
    fn drop(&mut self) {
        debug_println!("ArcShiftLight::drop: {:?}", self.item);
        drop_root_item(self.item)
    }
}



/// SAFETY:
/// If T is Sync, ArcShift<T> can also be Sync
unsafe impl<T:'static+Sync> Sync for ArcShift<T> {}

/// SAFETY:
/// If T is Send, ArcShift<T> can also be Send
unsafe impl<T:'static+Send> Send for ArcShift<T> {}

impl<T> Drop for ArcShift<T> {
    fn drop(&mut self) {
        verify_item(self.item);
        debug_println!("ArcShift::drop({:?})", self.item);
        let _t = unsafe { &*self.item }.next_and_state.load(Ordering::SeqCst);
        debug_println!("Check2: {:?} next: {:?}", self.item, _t);

        self.reload();
        let _t = unsafe { &*self.item }.next_and_state.load(Ordering::SeqCst);
        debug_println!("Check3: {:?} next: {:?}", self.item, _t);

        debug_println!("ArcShift::drop({:?}) - reloaded", self.item);
        drop_item(self.item);
        debug_println!("ArcShift::drop({:?}) DONE", self.item);
    }
}

/// Align 4 is needed, since we store flags in the lower 2 bits of the ItemHolder-pointers
#[repr(align(4))]
#[repr(C)]
struct ItemHolder<T:'static> {
    magic1: std::sync::atomic::AtomicU64,
    next_and_state: atomic::AtomicPtr<ItemHolder<T>>,
    refcount: atomic::AtomicUsize,
    payload: T,
    magic2: std::sync::atomic::AtomicU64,
}


impl<T:'static> ItemHolder<T> {
    fn verify(&self) {
        let magic1 = self.magic1.load(Ordering::SeqCst);
        let magic2 = self.magic2.load(Ordering::SeqCst);
        if magic1>>16 != 0xbeefbeefbeef || magic1&0xffff < 0x8000 {
            eprintln!("Internal error - bad magic1 in {:?}: {} ({:x})", self as *const ItemHolder<T>, magic1, magic1);
            println!("Backtrace: {}", Backtrace::capture());
            abort();
        }
        if magic2>>16 != 0x123412341234 || magic2&0xffff < 0x8000 {
            eprintln!("Internal error - bad magic2 in {:?}: {} ({:x})", self as *const ItemHolder<T>, magic2, magic2);
            println!("Backtrace: {}", Backtrace::capture());
            abort();
        }
        let diff = (magic1&0xffff) as isize - (magic2&0xffff) as isize;
        if diff != 0 {
            eprintln!("Internal error - bad magics in {:?}: {} ({:x}) and {} ({:x})", self as *const ItemHolder<T>, magic1, magic1, magic2, magic2);
            println!("Backtrace: {}", Backtrace::capture());
            abort();
        }
        self.magic1.fetch_add(1, Ordering::SeqCst);
        self.magic2.fetch_add(1, Ordering::SeqCst);
    }
}
fn verify_item<T>(ptr: *const ItemHolder<T>) {
    let x = undecorate(ptr);
    if x.is_null() {
        return;
    }
    unsafe{&*x}.verify()
}

impl<T> Drop for ItemHolder<T> {
    fn drop(&mut self) {
        self.verify();
        debug_println!("ItemHolder<T>::drop {:?}", self as *const ItemHolder<T>);
        self.magic1 = std::sync::atomic::AtomicU64::new(0xDEADDEA1DEADDEA1);
        self.magic2 = std::sync::atomic::AtomicU64::new(0xDEADDEA2DEADDEA2);
    }
}

#[repr(u8)]
#[derive(Clone,Copy,Debug,PartialEq, Eq)]
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
    Dropped = 3
}

fn decorate<T>(ptr: *const ItemHolder<T>, e: ItemStateEnum) -> *const ItemHolder<T> {
    let curdecoration = (ptr as usize)&3;
    ((ptr as *const u8).wrapping_offset( (e as isize)-(curdecoration as isize) )) as *const ItemHolder<T>
}

fn state_change<T>(ptr: &atomic::AtomicPtr<ItemHolder<T>>, prev: *const ItemHolder<T>, from: Option<ItemStateEnum>, to: ItemStateEnum) -> bool {
    if get_state(prev) != from {
        eprintln!("Internal error - wrong start-state: {:?}", get_state(prev));
        abort();
    }
    let to_decorated = decorate(prev, to);
    let t = ptr.compare_exchange(prev as *mut _, to_decorated as *mut _, Ordering::SeqCst, Ordering::SeqCst).is_ok();
    debug_println!("state_change {:?} -> {:?} (success: {:?})", prev, to_decorated, t);
    t
}

fn assert_is_undecorated<T>(ptr: *const ItemHolder<T>) {
    let raw = ptr as usize & 3;
    if raw != 0 {
        eprintln!("Internal error - unexpected decorated pointer");
        abort();
    }
}

fn undecorate<T>(cand: *const ItemHolder<T>) -> *const ItemHolder<T> {
    let raw = cand as usize & 3;
    if raw != 0 {
        ((cand as *const u8).wrapping_offset(-(raw as isize))) as *const ItemHolder<T>
    } else {
        cand
    }
}

fn get_state<T>(ptr: *const ItemHolder<T>) -> Option<ItemStateEnum> {
    if ptr.is_null() {
        return None;
    }
    //debug_println!("Getting state encoded in {:?}", ptr);
    let raw = ((ptr as usize) & 3) as u8;
    if raw == 0 {
        eprintln!("Encountered undecorated pointer in get_state!: {:?}", ptr);
        abort();
    }
    // SAFETY:
    // All values `0..=3` are valid ItemStateEnum.
    // And the bitmask produces a value `0..=3`
    Some( unsafe { std::mem::transmute::<u8, ItemStateEnum>( raw ) } )
}

fn is_dropped(state: Option<ItemStateEnum>) -> bool {
    matches!(state, Some(ItemStateEnum::Dropped))
}
fn is_superseded_by_tentative(state: Option<ItemStateEnum>) -> bool {
    matches!(state, Some(ItemStateEnum::SupersededByTentative))
}

impl<T:'static> Clone for ArcShift<T> {
    fn clone(&self) -> Self {
        debug_println!("ArcShift::clone({:?})", self.item);
        // SAFETY:
        // `self.item` is never null, and is always a valid pointer.
        // We always already have a refcount of at least 1 on entry to clone, and nothing
        // can take that away, since &mut self is needed to decrement refcount.
        let rescount = unsafe { (*self.item).refcount.fetch_add(MAX_ROOTS, atomic::Ordering::SeqCst) };
        debug_println!("Clone - adding count to {:?}, resulting in count {}", self.item, rescount + MAX_ROOTS);
        if rescount >= MAX_ARCSHIFT {
            eprintln!("Max number of ArcShift instances exceeded");
            abort();
        }
        ArcShift {
            item: self.item,
        }
    }
}
impl<T> Deref for ArcShift<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.shared_get()
    }
}

impl<T:'static> ArcShift<T> {

    fn drop_impl(mut self_item: *const ItemHolder<T>) {
        loop {
            if self_item.is_null() {
                return;
            }
            let count = unsafe { &*self_item }.refcount.fetch_sub(1, atomic::Ordering::SeqCst);
            if count == MAX_ROOTS {
                let new_item = undecorate(unsafe { &*self_item }.next_and_state.load(Ordering::SeqCst));
                drop_payload(self_item);
                self_item = new_item;
                atomic::spin_loop();
            } else {
                return;
            }
        }
    }

    /*fn drop_impl(mut self_item: *const ItemHolder<T>, drop_last_too: bool) -> Option<(bool,*const ItemHolder<T>)> {

        let mut hold_strength = MAX_ROOTS;
        loop {
            if self_item.is_null() {
                return None;
            }

            verify_item(self_item);

            debug_println!("drop_impl accessing {:?}", self_item);
            // SAFETY:
            // self.item is always a valid pointer
            let rawcand: *const ItemHolder<T> = unsafe { &*self_item }.next_and_state.load(atomic::Ordering::SeqCst) as *const ItemHolder<T>;

            debug_println!("rawcand is {:?}", rawcand);

            if undecorate(rawcand).is_null() && !drop_last_too {
                debug_println!("drop_impl producing owned pointer {:?}, with hold strength {}", self_item, hold_strength);
                if hold_strength != MAX_ROOTS {
                    unsafe { (&*self_item).refcount.fetch_add(MAX_ROOTS-hold_strength, atomic::Ordering::SeqCst) };
                }
                return Some((true,undecorate(self_item)));
            }

            let mut candstate = get_state(rawcand);
            let mut cand = undecorate(rawcand);
            {
                if !cand.is_null() && candstate == Some(ItemStateEnum::SupersededByTentative) {
                    verify_item(self_item);
                    //verify_item(cand);
                    debug_println!("candstate indicates tentative - make {:?} be solid.", self_item);
                    // SAFETY:
                    // Todo: self_item is valid pointer
                    match unsafe { &*self_item }.next_and_state.compare_exchange(rawcand as *mut _, decorate(cand, ItemStateEnum::Superseded) as *mut _, Ordering::SeqCst, Ordering::SeqCst) {
                        Ok(_) => {
                            verify_item(cand);
                            verify_item(rawcand);
                            debug_println!("drop_impl - Replacing {:?} with {:?} for {:?}", rawcand, decorate(cand, ItemStateEnum::Superseded), self_item);
                            candstate = Some(ItemStateEnum::Superseded);
                        }
                        Err(_) => {
                            atomic::spin_loop();
                            continue;
                        }
                    }
                }
                verify_item(cand);


                // SAFETY:
                // `cand` is always a valid pointer
                // We need to actually add to its refcount here, since otherwise some other
                // thread could come and do the same sort of cleanup we're doing, and it could
                // overtake us and actually free 'cand' immediately after we've decremented
                // our count for `self.item`
                if !cand.is_null() {
                    verify_item(cand);
                    let _candcount = unsafe { (*cand).refcount.fetch_add(MAX_ROOTS, atomic::Ordering::SeqCst) };
                    debug_println!("Cand {:?} count: {}", cand, _candcount);
                }


                verify_item(self_item);
                // SAFETY:
                // `self.item` is always a valid pointer
                let count = unsafe { &*self_item }.refcount.fetch_sub(1, atomic::Ordering::SeqCst);
                verify_item(self_item);

                //Thread 2 can be here, when thread 3 runs to end of this fun
                debug_println!("1-st step fetch sub post self.item count {} (prev: {}) of {:?} (hold strength: {})", count as isize - (1 as isize), count, self_item, hold_strength);
                assert!(count >= hold_strength && count < 1_000_000_000_000);
                if count == hold_strength {
                    if cand.is_null() {
                        // SAFETY:
                        // `self_item` is always a valid pointer.
                        // We need to reload self_item.next_and_state here, since some other thread may have written
                        // a value to it in the few lines above. This is safe to do, without any race conditions,
                        // since in this place in the code there are no longer any other owners.
                        verify_item(self_item);
                        let reloaded_cand = unsafe{&*self_item}.next_and_state.load(Ordering::SeqCst);
                        debug_println!("Reloaded cand for {:?} was: {:?}", self_item, reloaded_cand);
                        if !reloaded_cand.is_null() {
                            //rawcand = reloaded_cand; never used
                            cand = undecorate(reloaded_cand);
                            //candstate = get_state(reloaded_cand); never used
                            verify_item(cand);
                            let _candcount = unsafe { (*cand).refcount.fetch_add(MAX_ROOTS, atomic::Ordering::SeqCst) };
                            debug_println!("Reloaded cand {:?} count: {}", cand, _candcount);
                        }
                    }
                    debug_println!("Actual drop of ItemHolder {:?} (cand = {:?})", self_item, cand);
                    if !cand.is_null() {
                        // SAFETY:
                        // `cand` is always a valid pointer
                        // Here we're removing the count that was provided by the `self.item` object that we're doing to drop below
                        verify_item(cand);
                        let dbg = unsafe { (*cand).refcount.fetch_sub(MAX_ROOTS, atomic::Ordering::SeqCst) }; //We know this can't bring the count to 0, so can be Relaxed
                        assert!(dbg >= MAX_ROOTS && dbg < 1_000_000_000_000);
                    }
                    // refcount is actually going to be 1 here, not 0, but it doesn't matter since the object is deallocated anyway
                    verify_item(self_item);
                    drop_payload(self_item)

                } else {
                    verify_item(self_item);

                    if hold_strength > 1 && count < 2*MAX_ROOTS && !is_dropped(candstate) && !cand.is_null() { // we never drop the last in the chain (where cand is null)
                        debug_println!("early drop check count: {}", count);
                        // SAFETY:
                        // self.item is always a valid pointer
                        verify_item(self_item);
                        if !state_change( &unsafe { &*self_item }.next_and_state, rawcand, get_state(rawcand), ItemStateEnum::Dropped) {
                            debug_println!("Change-state to dropped raced for {:?}", self_item);
                            if !cand.is_null() {
                                verify_item(cand);
                                let _candcount = unsafe { (*cand).refcount.fetch_sub(MAX_ROOTS, atomic::Ordering::SeqCst) };
                                assert!(_candcount < 1_000_000_000_000);
                                debug_println!("Cand count: {}", _candcount);
                            }

                            verify_item(self_item);
                            unsafe { &*self_item }.refcount.fetch_add(1, atomic::Ordering::SeqCst);

                            // SAFETY:
                            // `self.item` is always a valid pointer
                            atomic::spin_loop();
                            continue;
                        }
                        // If we get here, we've changed the self.item - state (stored in self.item.next) to 'Dropped'.
                        // No-one else can do this change.

                        // If this triggers, then this is the last place where a non ArcShiftLight reference
                        // existed

                        // * All other instances must have been ArcShiftLight at the time we did 'refcount.fetch_sub'
                        // * One risk here is that ArcShiftLight::clone could have been called since then
                        // * But 'clone' can check after-the-fact if there is a 'next', and if there is,
                        //   it can reload, so even in this case it should be safe to take the payload
                        // * A second risk is that the remaining ArcShiftLight instances can be dropped.
                        //   However, this can be mitigated by deciding that ArcShiftLight instances can't drop
                        //   values of items with next-pointers. The responsibility then falls to ArcShift
                        //   to always drop values when it leaves an item with a next-pointer.

                        // SAFETY:
                        // `cand` is always a valid pointer
                        // Here we're downgrading the count that was provided by the `self.item` object that we're going to drop below.
                        // Previously, it was a 'strong' reference (value MAX_ROOTS), but now it's going to be a weak one (value 1).
                        if !cand.is_null() {
                            let dbg = unsafe { (*cand).refcount.fetch_sub(MAX_ROOTS - 1, atomic::Ordering::SeqCst) }; //We know this can't bring the count to 0, so can be Relaxed
                            debug_println!("Weakening hold on {:?} to {}", cand, dbg - (MAX_ROOTS-1));
                            assert!(dbg >= MAX_ROOTS && dbg < 1_000_000_000_000);
                        }

                        // SAFETY:
                        // self.item is always a valid pointer.
                        // because count before decrease was < 2*MAX_ROOTS, we must have been the last ArcShift-instance
                        // holding the heap block. Since there is a non-null 'next' pointer, no more ArcShift instances will be created.
                        // it is thus safe to take a reference to 'moved_out' and 'payload' fields, since they
                        // are never used by ArcShiftLight.
                        // SAFETY:
                        // self.item is always a valid pointer
                        verify_item(self_item);
                        let payload_item_mut = unsafe { &mut *addr_of_mut!((*(self_item as *mut ItemHolder<T>)).payload) };

                        // SAFETY:
                        // Since our compare-exchange higher up succeeded,
                        // we are the only ones allowed to drop the payload value
                        unsafe { std::ptr::drop_in_place(payload_item_mut) }
                        debug_println!("Early drop optimization active! for: {:?}", self_item);
                    } else {
                        //compile_error!("This probably leaks memory. Try to test better.")
                    }

                    if hold_strength > 1 {
                        verify_item(self_item);
                        let newcount = unsafe { &*self_item }.refcount.fetch_sub(MAX_ROOTS-1, atomic::Ordering::SeqCst);
                        debug_println!("Second reduction count: {} (end: {})", newcount, newcount - (MAX_ROOTS-1));
                        assert!(newcount >= MAX_ROOTS-1 &&  newcount < 1_000_000_000_000);
                        if newcount == MAX_ROOTS-1 {

                            // SAFETY:
                            // `self_item` is always a valid pointer.
                            // We need to reload self_item.next_and_state here, since some other thread may have written
                            // a value to it in the few lines above. This is safe to do, without any race conditions,
                            // since in this place in the code there are no longer any other owners.
                            verify_item(self_item);
                            let reloaded_cand = unsafe{&*self_item}.next_and_state.load(Ordering::SeqCst);
                            debug_println!("Reloaded cand for {:?} was: {:?}", self_item, reloaded_cand);
                            if !reloaded_cand.is_null() {
                                if cand.is_null() {
                                    verify_item(reloaded_cand);
                                    let _candcount = unsafe { (*undecorate(reloaded_cand)).refcount.fetch_add(MAX_ROOTS, atomic::Ordering::SeqCst) };
                                    debug_println!("Reloaded cand {:?} count: {}", cand, _candcount);
                                }
                                cand = undecorate(reloaded_cand);
                                candstate = get_state(reloaded_cand);
                                //rawcand = reloaded_cand; never used
                            }
                            verify_item(self_item);

                            debug_println!("Candstate: {:?}, state valid for {:?}", candstate, self_item);
                            if candstate == Some(ItemStateEnum::Dropped) {
                                hold_strength = 1;
                            } else {
                                hold_strength = MAX_ROOTS;
                            }

                            if !cand.is_null() {
                                verify_item(cand);
                                let _dbg = unsafe { (*cand).refcount.fetch_sub(MAX_ROOTS, atomic::Ordering::SeqCst) }; //Can't reach 0, self.item will point to this soon
                                assert!(_dbg > 0);
                                assert!(_dbg < 1_000_000_000_000);
                                debug_println!("For: {:?}, reduce next(={:?})-refcount to {} (reduction by {})", self_item, cand, _dbg-(MAX_ROOTS), MAX_ROOTS);
                            }
                            drop_payload(self_item);
                        } else {
                            debug_println!("Can't continue drop - we're not last for {:?}", self_item);
                            if hold_strength != MAX_ROOTS {
                                unsafe { (&*self_item).refcount.fetch_add(MAX_ROOTS-hold_strength, atomic::Ordering::SeqCst) };
                            }
                            return Some((false, self_item));
                        }
                    } else {
                        debug_println!("Can't continue drop - we've weak hold and didn't initially get refcount to 0 {:?}", self_item);

                        if hold_strength != MAX_ROOTS {
                            unsafe { (&*self_item).refcount.fetch_add(MAX_ROOTS-hold_strength, atomic::Ordering::SeqCst) };
                        }
                        return Some((false, self_item));
                    }
                }
                debug_println!("Doing assign, replacing {:?} with {:?}, hold-strength: {}", self_item, cand, hold_strength);
                self_item = cand;
            }
            atomic::spin_loop();
        }


    }*/

    /// Create a new ArcShift, containing the given type.
    pub fn new(payload: T) -> ArcShift<T> {
        let item = ItemHolder {
            magic1: std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111),
            magic2: std::sync::atomic::AtomicU64::new(0x1234123412348111),
            payload,
            next_and_state: atomic::AtomicPtr::default(),
            refcount: atomic::AtomicUsize::new(MAX_ROOTS),
        };
        let cur_ptr = Box::into_raw(Box::new(item));
        ArcShift {
            item: cur_ptr,
        }
    }
    /// Basically the same as doing ArcShift::new, but avoids copying the contents of 'input'
    /// to the stack, even as a temporary variable.
    pub fn from_box(input: Box<T>) -> ArcShift<T> {


        let input_ptr = Box::into_raw(input);

        let layout = Layout::new::<MaybeUninit<ItemHolder<T>>>();
        // SAFETY:
        // std::alloc::alloc requires the allocated layout to have a nonzero size. This
        // is fulfilled, since ItemHolder is non-zero sized even if T is zero-sized.
        // The returned memory is uninitialized, but we will initialize the required parts of it
        // below.
        let result_ptr = unsafe { std::alloc::alloc(layout) as *mut ItemHolder<T>};

        // SAFETY:
        // The copy is safe because MaybeUninit<ItemHolder<T>> is guaranteed to have the same
        // memory layout as ItemHolder<T>, so we're just copying a value of T to a new location.
        unsafe { addr_of_mut!((*result_ptr).payload).copy_from(input_ptr, 1); }
        // SAFETY:
        // next is just an AtomicPtr-type, for which all bit patterns are valid.
        unsafe { addr_of_mut!((*result_ptr).next_and_state).write(atomic::AtomicPtr::default()); }
        // SAFETY:
        // next is just an AtomicUsize-type, for which all bit patterns are valid.
        unsafe { addr_of_mut!((*result_ptr).refcount).write(atomic::AtomicUsize::new(MAX_ROOTS)); }

        unsafe { addr_of_mut!((*result_ptr).magic1).write(std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111)); }
        unsafe { addr_of_mut!((*result_ptr).magic2).write(std::sync::atomic::AtomicU64::new(0x1234123412348111)); }


        // SAFETY:
        // input_ptr is a *mut T that has been created from a Box<T>.
        // Converting it to a Box<MaybeUninit<T>> is safe, and will make sure that any drop-function
        // of T is not run. We must not drop T here, since we've moved it to the 'result_ptr'.
        let _t : Box<MaybeUninit<T>> = unsafe { Box::from_raw(input_ptr as *mut MaybeUninit<T>) }; //Free the memory, but don't drop 'input'
        ArcShift {
            item: result_ptr as *const ItemHolder<T>
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

        // SAFETY:
        // We always have refcount for self.item, and it is guaranteed valid
        if unsafe{&*self.item}.refcount.load(Ordering::SeqCst) == MAX_ROOTS {
            // SAFETY:
            // We always have refcount for self.item, and it is guaranteed valid
            Some(unsafe { &mut (*(self.item as *mut ItemHolder<T>)).payload })
        } else {
            None
        }
    }

    /// Try to move the value out of the ArcShift instance.
    /// This only succeeds if the self instance is the only instance
    /// holding the value.
    /*pub fn try_into_inner(mut self) -> Option<T> {
        self.reload();

        debug_println!("Try_into_inner");
        let retval = drop_item_ret(self.item);
        std::mem::forget(self);

        retval
    }*/

    /// Update the contents of this ArcShift, and all other instances cloned from this
    /// instance. The next time such an instance of ArcShift is dereferenced, this
    /// new value will be returned.
    ///
    /// WARNING!
    /// Calling this function does *not* cause the old value to be dropped before
    /// the new value is stored. The old instance of T is dropped when the last
    /// ArcShift instance upgrades to the new value. This update happens only
    /// when the instance is dereferenced, or the 'update' method is called.
    pub fn update_shared(&self, new_payload: T) {
        let item = ItemHolder {
            magic1: std::sync::atomic::AtomicU64::new(0xbeefbeefbeef8111),
            magic2: std::sync::atomic::AtomicU64::new(0x1234123412348111),
            payload: new_payload,
            next_and_state: atomic::AtomicPtr::default(),
            refcount: atomic::AtomicUsize::new(MAX_ROOTS),
        };
        let new_ptr = Box::into_raw(Box::new(item));
        debug_println!("Upgrading {:?} -> {:?} ", self.item, new_ptr);
        verify_item(new_ptr);
        let mut candidate = self.item;
        verify_item(candidate);
        loop {
            // SAFETY:
            // 'candidate' is either 'self.item', or one of the pointers in the linked list
            // chain. Each node in the chain has a refcount on the next node in the chain,
            // and self.item has a refcount on the first node. So all the nodes are valid.
            verify_item(candidate);
            let curnext  = unsafe { &*candidate }.next_and_state.load(Ordering::SeqCst);
            verify_item(curnext);
            let expect = if is_superseded_by_tentative(get_state(curnext)) {
                curnext
            } else {
                null_mut()
            };
            verify_item(expect);
            verify_item(candidate);
            verify_item(new_ptr);

            // SAFETY:
            // Candidate is still valid here, see above
            // Write a dummy-pointer, so that we can detect if it has been actually used or not,
            // and clean up at next update if not.
            verify_item(candidate);
            match unsafe { &*candidate }.next_and_state.compare_exchange(expect, decorate(new_ptr, ItemStateEnum::SupersededByTentative) as *mut ItemHolder<T>, atomic::Ordering::SeqCst, atomic::Ordering::SeqCst) {
                Ok(_) => {
                    verify_item(expect);
                    verify_item(new_ptr);
                    // Update complete.
                    debug_println!("Did replace next of {:?} (={:?}) with {:?} ", candidate, expect, decorate(new_ptr, ItemStateEnum::SupersededByTentative));
                    if is_superseded_by_tentative(get_state(expect))
                    {
                        // If we get here, then the dummy-optimization, allowing us to reduce
                        // garbage when updates are never actually loaded, has been triggered.
                        debug_println!("Anti-garbage optimization was in effect, dropping {:?}", expect);
                        drop_item(undecorate(expect));
                    }
                    debug_println!("Update shared complete");
                    //loom::sync::atomic::fence(Ordering::SeqCst);
                    verify_item(candidate);
                    let _t = unsafe { &*candidate }.next_and_state.load(Ordering::SeqCst);
                    debug_println!("Check: {:?} next: {:?}", candidate, _t);
                    return;
                }
                Err(other) => {
                    verify_item(candidate);
                    verify_item(new_ptr);
                    verify_item(other);
                    if !is_superseded_by_tentative(get_state(other)) {
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
    /// Update the contents of this ArcShift, and all other instance cloned from this
    /// instance. The next time such an instance of ArcShift is dereferenced, this
    /// new value will be returned.
    ///
    /// WARNING!
    /// Calling this function does *not* cause the old value to be dropped before
    /// the new value is stored. The old instance of T is dropped when the last
    /// ArcShift instance upgrades to the new value. This update happens only
    /// when the instance is dereferenced, or the 'update' method is called.
    ///
    /// Note, this method, in contrast to 'upgrade_shared', actually does update
    /// the 'self' ArcShift-instance. This has the effect that if 'self' is the
    /// only remaining instance, the old value that is being replaced will be dropped
    /// before this function returns.
    pub fn update(&mut self, new_payload: T) {
        self.update_shared(new_payload);
        debug_println!("self.reload()");
        self.reload();
    }

    /// This function makes sure to update this instance of ArcShift to the newest
    /// value.
    /// Calling the regular 'get' already does this, so this is rarely needed.
    /// But if mutable access to a ArcShift is only possible at certain points in the program,
    /// it may be clearer to call 'force_update' at those points to ensure any updates take
    /// effect, compared to just calling 'get' and discarding the value.
    #[inline(never)]
    pub fn reload(&mut self) {
        verify_item(self.item);

        let mut new_self = self.item;
        verify_item(new_self);
        loop {
            verify_item(new_self);
            let next = unsafe{&*new_self}.next_and_state.load(Ordering::SeqCst);;
            if get_state(next) == Some(ItemStateEnum::SupersededByTentative) {
                match unsafe{&*new_self}.next_and_state.compare_exchange(next, decorate(next, Superseded) as *mut _, Ordering::SeqCst, Ordering::SeqCst) {
                    Ok(_) => {
                        atomic::spin_loop();
                        continue;
                    }
                    Err(_) => {
                        atomic::spin_loop();
                        continue;
                    }
                }
            }
            if next.is_null() {
                break;
            }
            verify_item(next);
            new_self = undecorate(next);
            atomic::spin_loop();
        }
        if new_self == self.item {
            return; //Nothing to do
        }

        unsafe{&*new_self}.refcount.fetch_add(MAX_ROOTS, Ordering::SeqCst);

        let mut cand = self.item;
        while cand != new_self {
            verify_item(cand);
            verify_item(new_self);
            let count = unsafe{&*cand}.refcount.fetch_sub(MAX_ROOTS, Ordering::SeqCst);
            if count == MAX_ROOTS {
                let newcand = undecorate( unsafe{&*cand}.next_and_state.load(Ordering::SeqCst) );
                drop_payload(cand);
                cand = newcand;
            } else {
                break;
            }
        }
        self.item = new_self;
    }

    fn shared_get_impl(&self) -> *const ItemHolder<T> {
        let mut next_self_item = self.item;
        loop {
            debug_println!("shared_get_impl loop {:?}", next_self_item);
            assert_is_undecorated(next_self_item);
            // SAFETY:
            // `next_self_item` is always a valid pointer, because each node in the chain
            // has increased refcount on the next node in the chain.
            // The logic we're doing here is a part of the optimization which makes it possible to
            // do update multiple times in succession without ever introducing lingering garbage.
            // This is done by setting the LSB of the pointer to the new ItemHolder, marking it
            // as a dummy. At first actual use, the dummy bit is cleared (using `de_dummify`).
            verify_item(next_self_item);
            let cand: *const ItemHolder<T> = unsafe { &*next_self_item }.next_and_state.load(atomic::Ordering::SeqCst) as *const ItemHolder<T>;
            if cand.is_null() {
                break;
            }
            let cand = if is_superseded_by_tentative(get_state(cand)) {
                let fixed_cand = undecorate(cand); //Upgrade the item-pointer to a non-dummy, which will be written to memory below
                // SAFETY:
                // the .next pointer is always non-null and valid.
                // It is kept alive by the previous node always having a reference count on it (there can also be counts from ArcShift- and ArcShiftLight-instances)
                match unsafe { &*next_self_item }.next_and_state.compare_exchange(cand as *mut _, decorate(fixed_cand, ItemStateEnum::Superseded) as *mut _, atomic::Ordering::SeqCst, atomic::Ordering::SeqCst) {
                    Ok(_cand) => {
                        verify_item(fixed_cand);
                        debug_println!("Did replace {:?} with {:?} in {:?}", cand, fixed_cand, next_self_item);
                        fixed_cand
                    }
                    Err(_) => {
                        debug_println!("Failed cmpx {:?} -> {:?}", cand, fixed_cand);
                        atomic::spin_loop();
                        continue;
                    }
                }
            } else {
                cand
            };
            if !cand.is_null() {
                debug_println!("Doing assign");
                next_self_item = undecorate(cand);
            } else {
                break;
            }
            atomic::spin_loop();
        }
        // SAFETY:
        // `next_self_item` is always a valid pointer
        next_self_item
    }

    /// Like 'get', but only requires a &self, not &mut self.
    ///
    /// WARNING!
    /// This does not free old values after update. Call 'get' or 'force_update' to ensure this.
    /// Also, the run time of this method is proportional to the number of updates which have
    /// taken place without a 'get' or 'force_update'. The overhead is basically the price of
    /// one memory access per new available version - a few nanoseconds per version if data is in
    /// CPU cache.
    pub fn shared_get(&self) -> &T {
        debug_println!("Getting {:?}", self.item);
        // SAFETY:
        // `self.item` is always a valid pointer
        let cand: *const ItemHolder<T> = unsafe { &*self.item }.next_and_state.load(atomic::Ordering::Relaxed) as *const ItemHolder<T>;
        if !cand.is_null() {
            debug_println!("Update to {:?} detected", cand);
            return &unsafe {&*self.shared_get_impl()}.payload;
        }
        debug_println!("Returned payload for {:?}", self.item);
        // SAFETY:
        // `self.item` is always a valid pointer
        &unsafe { &*self.item }.payload
    }

    /// Return the value pointed to.
    ///
    /// This method is very fast, basically the speed of a regular pointer, unless
    /// the value has been modified by calling one of the update-methods.
    ///
    /// Note that this method requires 'mut self'. The reason 'mut' self is needed, is because
    /// of implementation reasons, and is what makes ArcShift 'get' very fast, while still
    /// allowing modification.
    #[inline(always)]
    pub fn get(&mut self) -> &T {
        debug_println!("Getting {:?}", self.item);
        // SAFETY:
        // `self.item` is always a valid pointer
        let cand: *const ItemHolder<T> = unsafe { &*self.item }.next_and_state.load(atomic::Ordering::Relaxed) as *const ItemHolder<T>;
        if !cand.is_null() {
            debug_println!("Update to {:?} detected", cand);
            self.reload();
        }
        debug_println!("Returned payload for {:?}", self.item);
        // SAFETY:
        // `self.item` is always a valid pointer
        &unsafe { &*self.item }.payload
    }

    /// Create an instance of ArcShiftLight, pointing to the same value as 'self'.
    ///
    /// WARNING!
    /// A maximum of 524287 ArcShiftLight-instances can be created for each value.
    /// An attempt to create more instances than this will fail with a panic.
    pub fn make_light(&self) -> ArcShiftLight<T> {
        let mut curitem = self.item;
        loop {
            // SAFETY:
            // curitem is a pointer reachable through the next-chain from self.item,
            // and is thus a valid pointer.
            let next = unsafe {&*curitem}.next_and_state.load(Ordering::SeqCst);
            if !next.is_null() {
                curitem = undecorate(next);
                atomic::spin_loop();
                continue;
            }

            // SAFETY:
            // curitem is a pointer reachable through the next-chain from self.item,
            // and is thus a valid pointer.
            let count = unsafe {&*curitem}.refcount.load(Ordering::SeqCst);
            if (count&(MAX_ROOTS-1)) == MAX_ROOTS - 1 {
                panic!("Maximum number of ArcShiftLight-instances has been reached.");
            }
            // SAFETY:
            // curitem is a pointer reachable through the next-chain from self.item,
            // and is thus a valid pointer.
            if unsafe {&*curitem}.refcount.compare_exchange(count, count + 1, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                atomic::spin_loop();
                continue;
            }
            break;
        }
        ArcShiftLight {
            item: curitem,
        }
    }



    /// This is like 'get', but never upgrades the pointer.
    /// This means that any new value supplied using one of the update methods will not be
    /// available.
    /// This method should be ever so slightly faster than regular 'get'.
    ///
    /// WARNING!
    /// You should probably not be using this method. If acquiring a non-upgraded value is
    /// acceptable, you should consider just using regular 'Arc'.
    /// One usecase is if you can control locations where an update is required, and arrange
    /// for 'mut self' to be possible at those locations.
    /// But in this case, it might be better to just use 'get' and store the returned pointer
    /// (it has a similar effect).
    #[inline(always)]
    pub fn shared_non_reloading_get(&self) -> &T {
        // SAFETY:
        // `self.item` is always a valid pointer
        &unsafe { &*self.item }.payload
    }


}


/// SAFETY:
/// The 'ptr' must be a valid pointer to an ItemHolder heap item that
/// is okay to drop.
fn drop_payload<T:'static>(ptr: *const ItemHolder<T>) {
    // SAFETY:
    // ptr must be valid, since this is a precondition for using this method.
    // This method does not need unsafe, since it is private, and the scope for
    // unsafety is within the entire crate.
    verify_item(ptr);

    if is_dropped(get_state(unsafe{&*addr_of!((*ptr).next_and_state)}.load( Ordering::SeqCst))) {
        debug_println!("Dropping payload {:?}, but contents were already dropped", ptr);
        // SAFETY:
        // `ptr` is always a valid pointer.
        // At this position we've established that we can drop the pointee's box memory, but the
        // pointee value is already dropped.
        _ = unsafe { Box::from_raw(ptr as *mut MaybeUninit<ItemHolder<T>>) };
    } else {
        debug_println!("Dropping payload {:?}, including contents", ptr);
        // SAFETY:
        // `ptr` is always a valid pointer.
        // At this position we've established that we can drop the pointee.
        _ = unsafe { Box::from_raw(ptr as *mut ItemHolder<T>) };

    }
}

// SAFETY:
// 'ptr' must be a pointer to an ItemHolder that is fit to drop.
/*fn drop_payload_ret<T:'static>(ptr: *const ItemHolder<T>) -> Option<T> {
    // SAFETY:
    // ptr must be valid, since this is a precondition for using this method.
    // This method does not need unsafe, since it is private, and the scope for
    // unsafety is within the entire crate.
    if is_dropped(get_state(unsafe{&*addr_of!((*ptr).next_and_state)}.load( Ordering::SeqCst))) {
        // SAFETY:
        // `ptr` is always a valid pointer.
        // At this position we've established that we can drop the pointee's box memory, but the
        // pointee value is already dropped.
        _ = unsafe { Box::from_raw(ptr as *mut MaybeUninit<ItemHolder<T>>) };
        None
    } else {
        // SAFETY:
        // Pointer must be a valid pointer, this is required to call this method.
        let mut holder = *unsafe { Box::from_raw(ptr as *mut ItemHolder<T>) };
        let payload_ptr = (&mut holder.payload) as *mut T;
        let payload;
        // SAFETY:
        // payload_ptr is a payload to the memory region of 'holder' where the payload
        // is stored. This region remains live until the end of the lifetime of 'holder'.
        unsafe {
            payload = payload_ptr.read();
        }
        std::mem::forget(holder);
        Some(payload)
    }
}*/

// SAFETY:
// 'old_ptr' must be a valid ItemHolder-pointer.
fn drop_root_item<T>(old_ptr: *const ItemHolder<T>) {
    debug_println!("drop_root_item {:?} about to subtract 1", old_ptr);
    // SAFETY:
    // old_ptr must be a valid pointer, required by caller. 'drop_root_item' is not unsafe because
    // it is not public.
    let count = unsafe{ &*addr_of!( (*old_ptr).refcount ) } .fetch_sub(1, atomic::Ordering::SeqCst);
    assert!(count >= 1 && count < 1_000_000_000_000);
    debug_println!("Drop-item {:?}, post-count = {}", old_ptr, count-1);
    if count == 1 {
        debug_println!("Begin drop of {:?}", old_ptr);
        // SAFETY:
        // old_ptr must be a valid pointer
        let next = undecorate( unsafe {&*addr_of!((*old_ptr).next_and_state)}.load(atomic::Ordering::SeqCst) );
        if !next.is_null() {
            debug_println!("Actual drop of ItemHolder {:?} - recursing into {:?}", old_ptr, next);
            crate::drop_root_item(next);
        }
        debug_println!("Actual drop of ItemHolder {:?}", old_ptr);

        drop_payload(old_ptr);
    }
}
fn drop_item<T>(old_ptr: *const ItemHolder<T>) {
    verify_item(old_ptr);
    _ = ArcShift::drop_impl(old_ptr);
}
#[cfg(test)]
pub mod tests {
    #![allow(dead_code)]
    #![allow(unused_imports)]
    use std::alloc::{Layout};
    use std::fmt::Debug;
    use std::hash::Hash;
    use std::hint::black_box;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;
    use std::time::Duration;
    use crossbeam_channel::bounded;
    use super::*;

    use rand::{Rng, SeedableRng};
    use rand::prelude::StdRng;


    #[cfg(not(loom))]
    fn model(x: impl FnOnce()) {
        x()
    }
    #[cfg(loom)]
    fn model(x: impl Fn()+'static+Send+Sync) {
        loom::model(x)
    }


    #[test]
    fn simple_get() {
        model(|| {
            let mut shift = ArcShift::new(42u32);
            assert_eq!(*shift.get(), 42u32);
        })
    }

    #[test]
    fn simple_large() {
        model(|| {
            #[cfg(not(miri))]
            const SIZE:usize = 10_000_000;
            #[cfg(miri)]
            const SIZE:usize = 10_000;

            let layout = Layout::new::<MaybeUninit<[u64;SIZE]>>();

            let ptr : *mut [u64;SIZE] = unsafe { std::alloc::alloc(layout) } as *mut [u64;SIZE];
            for i in 0..SIZE {
                unsafe { *(&mut (*ptr)).get_unchecked_mut(i) = 42 };
            }
            let bigbox = unsafe { Box::from_raw(ptr) };
            let mut shift = ArcShift::from_box(bigbox);
            let value = shift.get();
            assert_eq!(value[0], 42);
        })
    }
    #[test]
    fn simple_get2() {
        model(|| {
            let mut shift = ArcShift::from_box(Box::new(42u32));
            assert_eq!(*shift.get(), 42u32);
        })
    }
    #[test]
    fn simple_get3() {
        model(|| {
            let mut shift = ArcShift::new("hello".to_string());
            assert_eq!(shift.get(), "hello");
        })
    }
    #[test]
    fn simple_upgrade0() {
        model(||{
            let mut shift = ArcShift::new(42u32);
            assert_eq!(*shift.get(), 42u32);
            shift.update_shared(43);
            assert_eq!(*shift.get(), 43u32);
        });
    }

    #[test]
    fn simple_upgrade2() {
        model(||{
            let mut shift = ArcShift::new(42u32);
            assert_eq!(*shift.get(), 42u32);
            shift.update_shared(43);
            shift.update_shared(44);
            shift.update_shared(45);
            assert_eq!(*shift.get(), 45u32);
        });
    }

    /// A little helper struct that just keeps track of the number of live
    /// instances of it. This is used together with Loom to ensure there
    /// are no memory leaks due to race-conditions.
    struct InstanceSpy {
        x: std::sync::Arc<std::sync::atomic::AtomicUsize>
    }
    impl InstanceSpy {
        fn new(x: std::sync::Arc<std::sync::atomic::AtomicUsize>) -> InstanceSpy {

            let _temp = x.fetch_add(1, Ordering::Relaxed);
            debug_println!("++ InstanceSpy ++ {}", _temp + 1);
            InstanceSpy {
                x,
            }
        }
    }
    impl Drop for InstanceSpy {
        fn drop(&mut self) {
            let _prev = self.x.fetch_sub(1, Ordering::Relaxed);
            debug_println!("-- InstanceSpy -- {}", _prev - 1);
        }
    }
    struct InstanceSpy2 {
        x: std::sync::Arc<Mutex<HashSet<&'static str>>>,
        name: &'static str,
    }
    impl InstanceSpy2 {
        fn new(x: std::sync::Arc<Mutex<HashSet<&'static str>>>, name: &'static str) -> InstanceSpy2 {
            let mut guard = x.lock().unwrap();
            guard.insert(name);
            debug_println!("++ InstanceSpy ++ {:?} (added: {})", &*guard, name);
            drop(guard);
            InstanceSpy2 {
                x,
                name
            }
        }
    }
    impl Drop for InstanceSpy2 {
        fn drop(&mut self) {
            let mut guard = self.x.lock().unwrap();
            guard.remove(self.name);
            debug_println!("-- InstanceSpy -- {:?} - removed {}", &*guard, self.name);
        }
    }
    #[test]
    fn simple_upgrade3a1() {
        model(||{
            let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let shift = ArcShiftLight::new(InstanceSpy::new(count.clone()));

            debug_println!("==== running shift.get() = ");
            let mut arc = shift.get();
            debug_println!("==== running arc.update() = ");
            arc.update(InstanceSpy::new(count.clone()));

            debug_println!("==== Instance count: {}", count.load(Ordering::SeqCst));
            assert_eq!(count.load(Ordering::SeqCst), 1); // The 'ArcShiftLight' should *not* keep any version alive
            debug_println!("==== drop arc =");
            drop(arc);
            assert_eq!(count.load(Ordering::SeqCst), 1);
            debug_println!("==== drop shiftroot =");
            drop(shift);
            assert_eq!(count.load(Ordering::SeqCst), 0);

        });
    }
    #[test]
    fn simple_upgrade3a0() {
        model(||{
            let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let shift = ArcShiftLight::new(InstanceSpy::new(count.clone()));
            let mut arc = shift.get();
            for _ in 0..10 {
                arc.update(InstanceSpy::new(count.clone()));
                debug_println!("Instance count: {}", count.load(Ordering::SeqCst));
                assert_eq!(count.load(Ordering::Relaxed), 1); // The 'ArcShiftLight' should *not* keep any version alive
            }
            drop(arc);
            assert_eq!(count.load(Ordering::SeqCst), 1);
            drop(shift);
            assert_eq!(count.load(Ordering::SeqCst), 0);

        });
    }
    #[test]
    fn simple_upgrade3b() {
        model(||{
            let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let shift = ArcShiftLight::new(InstanceSpy::new(count.clone()));
            let mut arc = shift.get();
            for _ in 0..10 {
                arc.update(InstanceSpy::new(count.clone()));
                black_box(arc.clone());
                debug_println!("Instance count: {}", count.load(Ordering::SeqCst));
                assert_eq!(count.load(Ordering::Relaxed), 1); // The 'ArcShiftLight' should *not* keep any version alive
            }
            drop(arc);
            assert_eq!(count.load(Ordering::Relaxed), 1);
            drop(shift);
            assert_eq!(count.load(Ordering::Relaxed), 0);

        });
    }

    #[test]
    fn simple_threading2() {

        model(||{
            let shift = ArcShift::new(42u32);
            let shift1 = shift.clone();
            let mut shift2 = shift1.clone();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(1_000_000).spawn(move||{

                shift1.update_shared(43);
                debug_println!("t1 dropping");
            }).unwrap();

            let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(1_000_000).spawn(move||{

                std::hint::black_box(shift2.get());
                debug_println!("t2 dropping");

            }).unwrap();
            _=t1.join().unwrap();
            _=t2.join().unwrap();
        });
    }
    #[test]
    fn simple_threading2b() {

        model(||{
            let shift1 = ArcShiftLight::new(42u32);
            let mut shift2 = shift1.get();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(1_000_000).spawn(move||{
                black_box(shift1.clone());
                debug_println!("t1 dropping");
            }).unwrap();

            let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(1_000_000).spawn(move||{

                std::hint::black_box(shift2.get());
                debug_println!("t2 dropping");
            }).unwrap();
            _=t1.join().unwrap();
            _=t2.join().unwrap();
        });
    }
    #[test]
    fn simple_threading2c() {

        model(||{
            let shift1 = ArcShiftLight::new(42u32);
            let mut shift2 = shift1.get();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(1_000_000).spawn(move||{
                black_box(shift1.clone());
                debug_println!("t1 dropping");
            }).unwrap();

            let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(1_000_000).spawn(move||{
                shift2.update(43);
                debug_println!("t2 dropping");
            }).unwrap();
            _=t1.join().unwrap();
            _=t2.join().unwrap();
        });
    }
    #[test]
    fn simple_threading3a() {
        model(|| {
            let shift1 = std::sync::Arc::new(ArcShift::new(42u32));
            let shift2 = std::sync::Arc::clone(&shift1);
            let mut shift3 = (*shift1).clone();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t1 =");
                shift1.update_shared(43);
                debug_println!(" = drop t1 =");
            }).unwrap();

            let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t2 =");
                let mut shift = (*shift2).clone();
                std::hint::black_box(shift.get());
                debug_println!(" = drop t2 =");
            }).unwrap();

            let t3 = atomic::thread::Builder::new().name("t3".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t3 =");
                std::hint::black_box(shift3.get());
                debug_println!(" = drop t3 =");
            }).unwrap();
            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            _ = t3.join().unwrap();
        });
    }
    #[test]
    fn simple_threading3b() {
        model(|| {
            let shift1 = std::sync::Arc::new(ArcShift::new(42u32));
            let shift2 = std::sync::Arc::clone(&shift1);
            let mut shift3 = (*shift1).clone();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t1 =");
                let mut shift = (*shift1).clone();
                std::hint::black_box(shift.get());
                debug_println!(" = drop t1 =");
            }).unwrap();

            let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t2 =");
                let mut shift = (*shift2).clone();
                std::hint::black_box(shift.get());
                debug_println!(" = drop t2 =");
            }).unwrap();

            let t3 = atomic::thread::Builder::new().name("t3".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t3 =");
                std::hint::black_box(shift3.get());
                debug_println!(" = drop t3 =");
            }).unwrap();
            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            _ = t3.join().unwrap();
        });
    }
    #[test]
    fn simple_threading3c() {
        model(|| {
            let shift1 = std::sync::Arc::new(ArcShift::new(42u32));
            let shift2 = std::sync::Arc::clone(&shift1);
            let shift3 = (*shift1).clone();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t1 =");
                std::hint::black_box(shift1.update_shared(43));
                debug_println!(" = drop t1 =");
            }).unwrap();

            let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t2 =");
                std::hint::black_box(shift2.update_shared(44));
                debug_println!(" = drop t2 =");
            }).unwrap();

            let t3 = atomic::thread::Builder::new().name("t3".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t3 =");
                std::hint::black_box(shift3.update_shared(45));
                debug_println!(" = drop t3 =");
            }).unwrap();
            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            _ = t3.join().unwrap();
        });
    }
    #[test]
    fn simple_threading4a() {
        model(|| {
            let shift1 = std::sync::Arc::new(ArcShift::new(42u32));
            let shift2 = std::sync::Arc::clone(&shift1);
            let shift3 = std::sync::Arc::clone(&shift1);
            let mut shift4 = (*shift1).clone();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t1 =");
                shift1.update_shared(43);
                debug_println!(" = drop t1 =");
            }).unwrap();

            let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t2 =");
                let mut shift = (*shift2).clone();
                std::hint::black_box(shift.get());
                debug_println!(" = drop t2 =");
            }).unwrap();

            let t3 = atomic::thread::Builder::new().name("t3".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t3 =");
                shift3.update_shared(44);
                debug_println!(" = drop t3 =");
            }).unwrap();
            let t4 = atomic::thread::Builder::new().name("t4".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t4 =");
                let t = std::hint::black_box(*shift4.get());
                debug_println!(" = drop t4 =");
                t
            }).unwrap();

            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            _ = t3.join().unwrap();
            let ret = t4.join().unwrap();
            assert!(ret == 42 || ret == 43 || ret == 44);
        });
    }

    #[test]
    fn simple_threading4b() {
        model(|| {
            let shift1 = std::sync::Arc::new(ArcShift::new(42u32));
            let shift2 = std::sync::Arc::clone(&shift1);
            let shift3 = std::sync::Arc::clone(&shift1);
            let shift4 = (*shift1).clone();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t1 =");
                shift1.update_shared(43);
                debug_println!(" = drop t1 =");
            }).unwrap();

            let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t2 =");
                let mut shift = (*shift2).clone();
                std::hint::black_box(shift.get());
                debug_println!(" = drop t2 =");
            }).unwrap();

            let t3 = atomic::thread::Builder::new().name("t3".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t3 =");
                shift3.update_shared(44);
                let t = std::hint::black_box((*shift3).shared_get());
                debug_println!(" = drop t3 =");
                return *t;
            }).unwrap();
            let t4 = atomic::thread::Builder::new().name("t4".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t4 =");
                //let t = std::hint::black_box(shift4.try_into_inner());
                let t = Some(std::hint::black_box(*shift4));
                debug_println!(" = drop t4 =");
                t
            }).unwrap();

            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            let ret3 = t3.join().unwrap();
            assert!(ret3 == 44 || ret3  == 43);
            let ret = t4.join().unwrap();
            assert!(ret == None || ret == Some(43) || ret == Some(44) || ret == Some(42));
        });
    }
    #[test]
    fn simple_threading4c() {
        model(|| {
            let count = std::sync::Arc::new(std::sync::Mutex::new(HashSet::new()));
            {
                let shift1 = std::sync::Arc::new(ArcShift::new(InstanceSpy2::new(count.clone(), "orig")));
                let shift2 = std::sync::Arc::clone(&shift1);
                let shift3 = std::sync::Arc::clone(&shift1);
                let shift4 = std::sync::Arc::clone(&shift1);
                let t =      std::sync::Arc::clone(&shift1);
                let count1 = count.clone();
                let count2 = count.clone();
                let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(1_000_00).spawn(move || {
                    debug_println!(" = On thread t1 = {:?}", std::thread::current().id());
                    shift1.update_shared(InstanceSpy2::new(count1.clone(), "t1val"));
                    debug_println!(" = drop t1 =");
                }).unwrap();

                let t2 = atomic::thread::Builder::new().name("t2a".to_string()).stack_size(1_000_00).spawn(move || {
                    debug_println!(" = On thread t2 = {:?}", std::thread::current().id());
                    let mut _shift = shift2; //.clone()
                    //std::hint::black_box(shift.get());
                    debug_println!(" = drop t2 =");
                }).unwrap();

                let t3 = atomic::thread::Builder::new().name("t3".to_string()).stack_size(1_000_00).spawn(move || {
                    debug_println!(" = On thread t3 = {:?}", std::thread::current().id());
                    shift3.update_shared(InstanceSpy2::new(count2.clone(), "t3val"));
                    //let _t = std::hint::black_box((*shift3).shared_get());
                    let dbgval = unsafe { &*shift3.item }.next_and_state.load(Ordering::SeqCst);
                    verify_item(shift3.item);
                    debug_println!("Checkt34c: {:?} next: {:?}", shift3.item, dbgval);
                    //unsafe { &*shift3.item }.next_and_state.store(dbgval, Ordering::SeqCst);
                    //loom::sync::atomic::fence(Ordering::SeqCst);
                    //loom::thread::yield_now();
                    debug_println!(" = drop t3 =");
                }).unwrap();
                let t4 = atomic::thread::Builder::new().name("t4".to_string()).stack_size(1_000_00).spawn(move || {
                    debug_println!(" = On thread t4 = {:?}", std::thread::current().id());
                    let shift4 = &*shift4;
                    verify_item(shift4.item);
                    debug_println!("Checkt44c: {:?} next: {:?}", shift4.item, unsafe { &*shift4.item }.next_and_state.load(Ordering::SeqCst));
                    let _t = std::hint::black_box(shift4);
                    debug_println!(" = drop t4 =");
                }).unwrap();

                _ = t1.join().unwrap();
                _ = t2.join().unwrap();
                _ = t3.join().unwrap();
                t4.join().unwrap();
            }
            debug_println!("All threads stopped");
            let guard = count.lock().unwrap();
            if guard.len() > 0 {
                panic!("Leaked: {:?}", &*guard);
            }
        });
    }
    #[test]
    fn simple_threading4d() {
        model(|| {
            let shift1 = std::sync::Arc::new(ArcShiftLight::new(42u32));
            let shift2 = shift1.get();
            let shift3 = shift1.get();
            let mut shift4 = shift1.get();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t1 =");
                black_box(shift1.get());
                debug_println!(" = drop t1 =");
            }).unwrap();

            let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t2 =");
                let mut shift = shift2.clone();
                std::hint::black_box(shift.get());
                debug_println!(" = drop t2 =");
            }).unwrap();

            let t3 = atomic::thread::Builder::new().name("t3".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t3 =");
                shift3.update_shared(44);
                let t = std::hint::black_box(shift3.shared_get());
                debug_println!(" = drop t3 =");
                return *t;
            }).unwrap();
            let t4 = atomic::thread::Builder::new().name("t4".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t4 =");
                let t = std::hint::black_box(shift4.try_get_mut().map(|x|*x));
                debug_println!(" = drop t4 =");
                t
            }).unwrap();

            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            let ret3 = t3.join().unwrap();
            assert!(ret3 == 44 || ret3  == 43);
            let ret = t4.join().unwrap();
            assert!(ret == None || ret == Some(43) || ret == Some(44) || ret == Some(42));
        });
    }
    enum PipeItem<T:'static> {
        Shift(ArcShift<T>),
        Root(ArcShiftLight<T>),
    }

    #[cfg(not(miri))]
    fn run_multi_fuzz<T:Clone+Hash+Eq+'static+Debug+Send+Sync>(rng: &mut StdRng,mut constructor: impl FnMut()->T) {
        let cmds = make_commands::<T>(rng, &mut constructor);
        let mut all_possible : HashSet<T> = HashSet::new();
        for cmd in cmds.iter() {
            if let FuzzerCommand::CreateUpdateArc(_, val) | FuzzerCommand::CreateArcRoot(_, val) = cmd {
                all_possible.insert(val.clone());
            }
        }
        let mut batches = Vec::new();
        let mut senders = vec![];
        let mut receivers = vec![];
        for _ in 0..3 {
            let (sender,receiver) = bounded::<PipeItem<T>>(cmds.len());
            senders.push(sender);
            receivers.push(receiver);
            batches.push(Vec::new());
        }
        for cmd in cmds {
            batches[cmd.batch() as usize].push(cmd);
        }

        let mut jhs = Vec::new();

        let senders = std::sync::Arc::new(senders);
        let all_possible : std::sync::Arc<HashSet<T>>= std::sync::Arc::new(all_possible);
        for (threadnr, (cmds, receiver)) in batches.into_iter().zip(receivers).enumerate() {
            let thread_senders = std::sync::Arc::clone(&senders);
            let thread_all_possible:std::sync::Arc<HashSet<T>> = std::sync::Arc::clone(&all_possible);
            let jh = atomic::thread::Builder::new().name(format!("thread{}", threadnr)).spawn(move||{

                let mut curval : Option<ArcShift<T>> = None;
                let mut curvalroot : Option<ArcShiftLight<T>> = None;
                for cmd in cmds {
                    if let Ok(val) = receiver.try_recv() {
                        match val {
                            PipeItem::Shift(shift) => {
                                curval = Some(shift);
                            }
                            PipeItem::Root(root) => {
                                curvalroot = Some(root);
                            }
                        }

                    }
                    match cmd {
                        FuzzerCommand::CreateUpdateArc(_, val) => {
                            if let Some(curval) = &mut curval {
                                curval.update_shared(val);
                            } else {
                                curval = Some(ArcShift::new(val));
                            }
                        }
                        FuzzerCommand::ReadArc { .. } => {
                            if let Some(curval) = curval.as_mut() {
                                let actual = curval.get();
                                if !thread_all_possible.contains(actual) {
                                    panic!("Unexpected value in thread {}, got {:?} which is not in {:?}", threadnr, actual, thread_all_possible);
                                }
                            }
                        }
                        FuzzerCommand::SharedReadArc { .. } => {
                            if let Some(curval) = curval.as_mut() {
                                let actual = curval.shared_get();
                                if !thread_all_possible.contains(actual) {
                                    panic!("Unexpected value in thread {}, got {:?} which is not in {:?}", threadnr, actual, thread_all_possible);
                                }
                            }
                        }
                        FuzzerCommand::CloneArc { from:_, to:_ } => {
                            if let Some(curval) = curval.as_mut() {
                                let cloned = curval.clone();
                                thread_senders[threadnr].send(PipeItem::Shift(cloned)).unwrap();
                            }
                        }
                        FuzzerCommand::DropArc(_) => {
                            curval = None;
                        }
                        FuzzerCommand::CreateArcRoot(_, val) => {
                            curvalroot = Some(ArcShiftLight::new(val));
                        }
                        FuzzerCommand::CloneArcRoot { .. } => {
                            if let Some(curvalroot) = curvalroot.as_mut() {
                                let cloned = curvalroot.clone();
                                thread_senders[threadnr].send(PipeItem::Root(cloned)).unwrap();
                            }
                        }
                        FuzzerCommand::PromoteRoot(_) => {
                            if let Some(root) = curvalroot.as_ref() {
                                curval = Some(root.get());
                            }
                        }
                        FuzzerCommand::DemoteArc(_) => {
                            if let Some(arc) = curval.as_ref() {
                                curvalroot = Some(arc.make_light());
                            }
                        }
                    }
                }
            }).unwrap();
            jhs.push(jh);

        }
        for jh in jhs {
            jh.join().unwrap();
        }

    }


    #[derive(Debug)]
    enum FuzzerCommand<T> {
        CreateUpdateArc(u8, T),
        CreateArcRoot(u8, T),
        ReadArc{arc: u8},
        SharedReadArc{arc: u8},
        CloneArc{from:u8,to:u8},
        CloneArcRoot{from:u8,to:u8},
        DropArc(u8),
        PromoteRoot(u8),
        DemoteArc(u8),
    }
    impl<T> FuzzerCommand<T> {
        fn batch(&self) -> u8 {
            match self {
                FuzzerCommand::CreateUpdateArc(chn, _) => {*chn}
                FuzzerCommand::ReadArc { arc} => {*arc}
                FuzzerCommand::SharedReadArc { arc } => {*arc}
                FuzzerCommand::CloneArc { from, .. } => {*from}
                FuzzerCommand::DropArc(chn) => {*chn}
                FuzzerCommand::CreateArcRoot(chn, _) => {*chn}
                FuzzerCommand::CloneArcRoot { from, .. } => {*from}
                FuzzerCommand::PromoteRoot(chn) => {*chn}
                FuzzerCommand::DemoteArc(chn) => {*chn}
            }
        }
    }
    fn run_fuzz<T:Clone+Hash+Eq+'static+Debug+Send+Sync>(rng: &mut StdRng, mut constructor: impl FnMut()->T) {
        let cmds = make_commands::<T>(rng, &mut constructor);
        let mut arcs: [Option<ArcShift<T>>; 3] = [();3].map(|_|None);
        let mut arcroots: [Option<ArcShiftLight<T>>; 3] = [();3].map(|_|None);
        debug_println!("Starting fuzzrun");
        for cmd in cmds {
            debug_println!("\n=== Applying cmd: {:?} ===", cmd);
            match cmd {
                FuzzerCommand::CreateUpdateArc(chn, val) => {
                    if let Some(arc) = &mut arcs[chn as usize] {
                        arc.update_shared(val);
                    } else {
                        arcs[chn as usize] = Some(ArcShift::new(val));
                    }
                }
                FuzzerCommand::ReadArc { arc } => {
                    if let Some(actual) = &mut arcs[arc as usize] {
                        let actual_val = actual.get();

                        std::hint::black_box(actual_val);
                    }
                }
                FuzzerCommand::SharedReadArc { arc } => {
                    if let Some(actual) = &mut arcs[arc as usize] {
                        let actual_val = actual.shared_get();

                        std::hint::black_box(actual_val);
                    }
                }
                FuzzerCommand::CloneArc { from, to } => {
                    let clone = arcs[from as usize].clone();
                    arcs[to as usize] = clone;
                }
                FuzzerCommand::DropArc(chn) => {
                    arcs[chn as usize] = None;
                }
                FuzzerCommand::CreateArcRoot(chn, val) => {
                    arcroots[chn as usize] = Some(ArcShiftLight::new(val));
                }
                FuzzerCommand::CloneArcRoot { from, to } => {
                    let clone = arcroots[from as usize].clone();
                    arcroots[to as usize] = clone;
                }
                FuzzerCommand::PromoteRoot(chn) => {
                    if let Some(root) = arcroots[chn as usize].as_ref() {
                        arcs[chn as usize] = Some(root.get());
                    }
                }
                FuzzerCommand::DemoteArc(chn) => {
                    if let Some(arc) = arcs[chn as usize].as_ref() {
                        arcroots[chn as usize] = Some(arc.make_light());
                    }
                }
            }

        }
        debug_println!("=== No more commands ===");

    }

    fn make_commands<T:Clone+Eq+Hash+Debug>(rng: &mut StdRng, constructor: &mut impl FnMut()->T) -> Vec<FuzzerCommand<T>> {

        let mut ret = Vec::new();
        for _x in 0..20  {
            match rng.gen_range(0..10) {
                0 => {
                    let chn = rng.gen_range(0..3);
                    let val = constructor();
                    ret.push(FuzzerCommand::CreateUpdateArc(chn, val.clone()));
                }
                1 => {
                    let chn = rng.gen_range(0..3);
                    ret.push(FuzzerCommand::ReadArc{arc:chn});
                }
                2 => { //Clone
                    let from = rng.gen_range(0..3);
                    let mut to = rng.gen_range(0..3);
                    if from == to {
                        to = (from + 1) % 3;
                    }

                    ret.push(FuzzerCommand::CloneArc{from, to});
                }
                3 => {
                    let chn = rng.gen_range(0..3);
                    ret.push(FuzzerCommand::DropArc(chn));
                }
                4 => {
                    let chn = rng.gen_range(0..3);
                    ret.push(FuzzerCommand::CreateArcRoot(chn, constructor()));
                }
                5 => {
                    let chn = rng.gen_range(0..3);
                    let val = constructor();
                    ret.push(FuzzerCommand::CreateArcRoot(chn, val.clone()));
                }
                6 => {
                    let from = rng.gen_range(0..3);
                    let mut to = rng.gen_range(0..3);
                    if from == to {
                        to = (from + 1) % 3;
                    }

                    ret.push(FuzzerCommand::CloneArcRoot{from, to});
                }
                7 => {
                    let chn = rng.gen_range(0..3);
                    ret.push(FuzzerCommand::PromoteRoot(chn));
                }
                8 => {
                    let chn = rng.gen_range(0..3);
                    ret.push(FuzzerCommand::DemoteArc(chn));
                }
                9 => {
                    let chn = rng.gen_range(0..3);
                    ret.push(FuzzerCommand::SharedReadArc{arc:chn});
                }
                _ => unreachable!()
            }
        }
        ret
    }

    #[test]
    #[cfg(not(miri))]
    fn generic_thread_fuzzing_all() {
        #[cfg(loom)]
        const COUNT: u64 = 5000;
        #[cfg(not(any(loom,miri)))]
        const COUNT: u64 = 10000;
        for i in 0..COUNT {
            model(move||{

                let mut rng = StdRng::seed_from_u64(i);
                let mut counter = 0u32;
                debug_println!("--- Seed {} ---", i);
                run_multi_fuzz(&mut rng, move || -> u32 {
                    counter += 1;
                    counter
                });
            });
        }
    }
    #[test]
    #[cfg(not(miri))]
    fn generic_fuzzing_all() {
        #[cfg(any(loom,miri))]
        const COUNT: u64 = 1000;
        #[cfg(not(any(loom,miri)))]
        const COUNT: u64 = 5000000;
        for i in 0..COUNT {
            model(move||{
                let mut rng = StdRng::seed_from_u64(i);
                let mut counter = 0u32;
                println!("Running seed {}", i);
                run_fuzz(&mut rng, move || -> u32 {
                    counter += 1;
                    counter
                });
            });
        };
    }
    #[test]
    fn generic_fuzzing_159() {
        let seed = 159;
        model(move||{
            let mut rng = StdRng::seed_from_u64(seed);
            let mut counter = 0u32;
            println!("Running seed {}", seed);
            run_fuzz(&mut rng, move || -> u32 {
                counter += 1;
                counter
            });
        })
    }
    #[test]
    fn generic_fuzzing_53014() {
        let seed = 53014;
        model(move||{
            let mut rng = StdRng::seed_from_u64(seed);
            let mut counter = 0u32;
            println!("Running seed {}", seed);
            run_fuzz(&mut rng, move || -> u32 {
                counter += 1;
                counter
            });
        })
    }
    #[test]
    fn generic_fuzzing_3817879() {
        let seed = 3817879;
        model(move||{
            let mut rng = StdRng::seed_from_u64(seed);
            let mut counter = 0u32;
            println!("Running seed {}", seed);
            run_fuzz(&mut rng, move || -> u32 {
                counter += 1;
                counter
            });
        })
    }
}
