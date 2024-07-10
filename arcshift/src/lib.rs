#![deny(warnings)]
#![deny(missing_docs)]
//! Crate containing an implementation of Arc that allows modifying
//! the value pointed to.
//!
//! The primary raison d'etre for ArcShift is that it provides very
//! little overhead over regular Arc for cloning, dropping and accessing
//! the value, while still allowing the value to be replaced.
//!
//! Specifically, accessing the value of an ArcShift only requires a single
//! atomic operation, of the least intrusive kind (Ordering::Relaxed). On x86_64,
//! this is the exact same machine operation as a regular memory access, and also
//! on arm it is not an expensive operation.
//!
//! ** Trade-offs **
//!
//! ArcShift achieves this at the expense of the following disadvantages:
//! * Modifying the value is more expensive than modifying RwLock<Arc<T>>
//! * When the value is modified, the next subsequent access is slower than an RwLock<Arc<T>>
//! * ArcShift instances must be owned (or be mutably accessible) to dereference.
//! * When modifying the value, the old version of the value lingers in memory until
//!   the last ArcShift has been updated. Such an update only happens when the ArcShift
//!   is accessed using an owned (or &mut) access.
//! * Note that the former limitation applies even if ArcShift::update is called multiple times
//!   in succession, without any instance actually using the value provided.
//!
//! The last limitation might seem unacceptable, but for many applications it is not
//! hard to make sure each thread/scope has its own instance of ArcShift. Remember that
//! cloning ArcShift is fast (the cost of a single Relaxed atomic increment).
//!

pub use std::collections::HashSet;
use std::ptr::{null_mut};
use std::sync::atomic::Ordering;

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
    pub use loom::thread;
    pub use loom::hint::spin_loop;

}

#[cfg(debug_assertions)]
macro_rules! debug_println {
    ($($x:tt)*) => { println!($($x)*) }
}


#[cfg(not(debug_assertions))]
macro_rules! debug_println {
    ($($x:tt)*) => ({})
}

/// Smart pointer with similar use case as std::sync::Arc, but with
/// the added ability to atomically replace the contents of the Arc.
pub struct ArcShift<T:'static+Send+Sync> {
    item: *const ItemHolder<T>,
}

unsafe impl<T:'static+Send+Sync> Sync for ArcShift<T> {}

unsafe impl<T:'static+Send+Sync> Send for ArcShift<T> {}

impl<T:Send+Sync> Drop for ArcShift<T> {
    fn drop(&mut self) {
        debug_println!("ArcShift::drop({:?})", self.item);
        drop_item(self.item);
    }
}
#[repr(align(2))]
struct ItemHolder<T:'static+Send+Sync> {
    payload: T,
    next: atomic::AtomicPtr<ItemHolder<T>>,
    refcount: atomic::AtomicUsize,
}
impl<T:Send+Sync> Drop for ItemHolder<T> {
    fn drop(&mut self) {
        debug_println!("ItemHolder<T>::drop {:?}", self as *const ItemHolder<T>);
    }
}


fn de_dummify<T:Send+Sync>(cand: *const ItemHolder<T>) -> *const ItemHolder<T> {
    if cand as usize & 1 == 1 {
        ((cand as *const u8).wrapping_offset(-1)) as *const ItemHolder<T>
    } else {
        cand
    }
}
fn is_dummy<T:Send+Sync>(cand: *const ItemHolder<T>) -> bool {
    ((cand as usize) & 1) == 1
}

impl<T:'static+Send+Sync> Clone for ArcShift<T> {
    fn clone(&self) -> Self {
        debug_println!("ArcShift::clone({:?})", self.item);
        let cur_item = self.item;
        let _rescount = unsafe { (*cur_item).refcount.fetch_add(1, atomic::Ordering::Relaxed) };
        debug_println!("Clone - adding count to {:?}, resulting in count {}", cur_item, _rescount + 1);
        ArcShift {
            item: cur_item,
        }
    }
}
impl<T:'static+Send+Sync> ArcShift<T> {
    /// Create a new ArcShift, containing the given type.
    pub fn new(payload: T) -> ArcShift<T> {
        let item = ItemHolder {
            payload,
            next: atomic::AtomicPtr::default(),
            refcount: atomic::AtomicUsize::new(1),
        };
        let cur_ptr = Box::into_raw(Box::new(item));
        ArcShift {
            item: cur_ptr,
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
    pub fn upgrade_shared(&self, new_payload: T) {
        let item = ItemHolder {
            payload: new_payload,
            next: atomic::AtomicPtr::default(),
            refcount: atomic::AtomicUsize::new(1),
        };
        let new_ptr = Box::into_raw(Box::new(item));
        debug_println!("Upgrading {:?} -> {:?} ", self.item, new_ptr);
        let mut candidate = self.item;
        loop {
            let curnext  = unsafe { &*candidate }.next.load(Ordering::SeqCst);
            let expect = if is_dummy(curnext) {
                curnext
            } else {
                null_mut()
            };
            match unsafe { &*candidate }.next.compare_exchange(expect, (new_ptr as *mut u8).wrapping_offset(1) as *mut ItemHolder<T>, atomic::Ordering::SeqCst, atomic::Ordering::SeqCst) {
                Ok(_) => {
                    // Update complete.
                    debug_println!("Did replace next of {:?} with {:?} ", candidate, new_ptr);
                    if is_dummy(expect)
                    {
                        drop_item(de_dummify(expect));
                    }
                    debug_println!("Update complete");
                    return;
                }
                Err(other) => {
                    debug_println!("Update not complete yet, spinning");
                    candidate = de_dummify(other);
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
        self.upgrade_shared(new_payload);
        self.get();
    }

    /// This function makes sure to update this instance of ArcShift to the newest
    /// value.
    /// Calling the regular 'get' already does this, so this is rarely needed.
    /// But if mutable access to a ArcShift is only possible at certain points in the program,
    /// it may be clearer to call 'force_update' at those points to ensure any updates take
    /// effect, compared to just calling 'get' and discarding the value.
    pub fn force_update(&mut self) {
        _ = self.get();
    }

    /// Return the value pointed to.
    ///
    /// This method is very fast, basically the speed of a regular pointer, unless
    /// the value has been modified by calling one of the update-methods.
    ///
    /// Note that this method requires 'mut self'. The reason 'mut' self is needed, is because
    /// of implementation reasons, and is what makes ArcShift 'get' very very fast, while still
    /// allowing modification.
    pub fn get(&mut self) -> &T {
        debug_println!("Getting {:?}", self.item);
        let cand: *const ItemHolder<T> = unsafe { &*self.item }.next.load(atomic::Ordering::Relaxed) as *const ItemHolder<T>;
        if !cand.is_null() {
            loop {
                debug_println!("Update to {:?} detected", cand);
                let cand: *const ItemHolder<T> = unsafe { &*self.item }.next.load(atomic::Ordering::SeqCst) as *const ItemHolder<T>;
                let fixed_cand = de_dummify(cand);
                let cand = match unsafe { &*self.item }.next.compare_exchange(cand as *mut _, fixed_cand as *mut _, atomic::Ordering::SeqCst, atomic::Ordering::SeqCst) {
                    Ok(cand) => {
                        de_dummify(cand)
                    }
                    Err(_) => {
                        debug_println!("Failed cmpx {:?} -> {:?}", cand, fixed_cand);
                        atomic::spin_loop();
                        continue;
                    }
                };
                if !cand.is_null() {
                    unsafe { (*cand).refcount.fetch_add(1, atomic::Ordering::Relaxed) };
                    let count = unsafe { &*self.item }.refcount.fetch_sub(1, atomic::Ordering::Acquire);
                    debug_println!("fetch sub received {}", count);
                    if count == 1 {
                        debug_println!("Actual drop of ItemHolder {:?}", self.item);
                        unsafe { (*cand).refcount.fetch_sub(1, atomic::Ordering::Relaxed) }; //We know this can't bring the count to 0, so can be Relaxed
                        _ = unsafe { Box::from_raw(self.item as *mut ItemHolder<T>) };
                    } else {
                        debug_println!("No drop of ItemHolder");
                    }
                    debug_println!("Doing assign");
                    self.item = cand;
                } else {
                    break;
                }
                atomic::spin_loop();
            }
        }
        debug_println!("Returned payload for {:?}", self.item);
        &unsafe { &*self.item }.payload
    }
    /// This is like 'get', but never upgrades the pointer.
    /// This means that new values supplied using one of the update methods will not be
    /// available.
    /// This method should be ever so slightly faster than regular 'get'.
    ///
    /// WARNING!
    /// You should probably not using this method. If acquiring a non-upgraded value is
    /// acceptable, you should consider just using regular 'Arc'.
    /// One usecase is if you can control locations where an update is required, and arrange
    /// for 'mut self' to be possible at those locations.
    /// But in this case, it might be better to just use 'get' and store the returned pointer
    /// (it has a similar effect).
    pub fn shared_nonupgrading_get(&self) -> &T {
        &unsafe { &*self.item }.payload
    }
}
fn drop_item<T:Send+Sync>(old_ptr: *const ItemHolder<T>) {
    debug_println!("drop_item {:?} about to subtract 1", old_ptr);
    let count = unsafe{&*old_ptr}.refcount.fetch_sub(1, atomic::Ordering::SeqCst);
    debug_println!("Drop-item {:?}, post-count = {}", old_ptr, count-1);
    if count == 1 {
        debug_println!("Begin drop of {:?}", old_ptr);
        //let next = *unsafe{&mut *(old_ptr as *mut ItemHolder<T>)}.next.get_mut();
        let next = de_dummify( unsafe {&*old_ptr}.next.load(atomic::Ordering::SeqCst) );
        if next.is_null() == false {
            debug_println!("Actual drop of ItemHolder {:?} - recursing into {:?}", old_ptr, next);
            drop_item(next);
        }
        debug_println!("Actual drop of ItemHolder {:?}", old_ptr);
        _ = unsafe { Box::from_raw(old_ptr as *mut ItemHolder<T>) };
    }
}


#[cfg(test)]
mod tests {
    use std::fmt::Debug;
    use std::hash::Hash;
    use crossbeam_channel::bounded;
    use super::*;

    use rand::{Rng, SeedableRng};
    use rand::prelude::StdRng;


    #[cfg(not(loom))]
    fn model(x: impl FnOnce()) {
        x()
    }
    #[cfg(loom)]
    fn model(x: impl Fn()+Send+Sync+'static) {
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
    fn simple_get2() {
        model(|| {
            let mut shift = ArcShift::new("hello".to_string());
            assert_eq!(shift.get(), "hello");
        })
    }
    #[test]
    fn simple_upgrade() {
        model(||{
            let mut shift = ArcShift::new(42u32);
            assert_eq!(*shift.get(), 42u32);
            shift.upgrade_shared(43);
            assert_eq!(*shift.get(), 43u32);
        });
    }

    #[test]
    fn simple_upgrade2() {
        model(||{
            let mut shift = ArcShift::new(42u32);
            assert_eq!(*shift.get(), 42u32);
            shift.upgrade_shared(43);
            shift.upgrade_shared(44);
            shift.upgrade_shared(45);
            assert_eq!(*shift.get(), 45u32);
        });
    }

    #[test]
    fn simple_threading2() {

        model(||{
            let shift = ArcShift::new(42u32);
            let shift1 = shift.clone();
            let mut shift2 = shift1.clone();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(1_000_000).spawn(move||{

                shift1.upgrade_shared(43);
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
    fn simple_threading3a() {
        model(|| {
            let shift1 = std::sync::Arc::new(ArcShift::new(42u32));
            let shift2 = std::sync::Arc::clone(&shift1);
            let mut shift3 = (*shift1).clone();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t1 =");
                shift1.upgrade_shared(43);
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
    fn simple_threading4() {
        model(|| {
            let shift1 = std::sync::Arc::new(ArcShift::new(42u32));
            let shift2 = std::sync::Arc::clone(&shift1);
            let shift3 = std::sync::Arc::clone(&shift1);
            let mut shift4 = (*shift1).clone();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(1_000_000).spawn(move || {
                debug_println!(" = On thread t1 =");
                shift1.upgrade_shared(43);
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
                shift3.upgrade_shared(44);
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


    fn run_multi_fuzz<T:Clone+Hash+Eq+'static+Debug+Send+Sync>(rng: &mut StdRng,mut constructor: impl FnMut()->T) {
        let cmds = make_commands::<T>(rng, &mut constructor);
        let mut all_possible : HashSet<T> = HashSet::new();
        for cmd in cmds.iter() {
            if let FuzzerCommand::CreateUpdateArc(_, val) = cmd {
                all_possible.insert(val.clone());
            }
        }
        let mut batches = Vec::new();
        let mut senders = vec![];
        let mut receivers = vec![];
        for _ in 0..3 {
            let (sender,receiver) = bounded(cmds.len());
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
                for cmd in cmds {
                    if let Ok(val) = receiver.try_recv() {
                        curval = Some(val);
                    }
                    match cmd {
                        FuzzerCommand::CreateUpdateArc(_, val) => {
                            if let Some(curval) = &mut curval {
                                curval.upgrade_shared(val);
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
                        FuzzerCommand::CloneArc { from:_, to:_ } => {
                            if let Some(curval) = curval.as_mut() {
                                let cloned = curval.clone();
                                thread_senders[threadnr].send(cloned).unwrap();
                            }
                        }
                        FuzzerCommand::DropArc(_) => {
                            curval = None;
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
        ReadArc{arc: u8},
        CloneArc{from:u8,to:u8},
        DropArc(u8)
    }
    impl<T> FuzzerCommand<T> {
        fn batch(&self) -> u8 {
            match self {
                FuzzerCommand::CreateUpdateArc(chn, _) => {*chn}
                FuzzerCommand::ReadArc { arc, .. } => {*arc}
                FuzzerCommand::CloneArc { from, .. } => {*from}
                FuzzerCommand::DropArc(chn) => {*chn}
            }
        }
    }
    fn run_fuzz<T:Clone+Hash+Eq+'static+Debug+Send+Sync>(rng: &mut StdRng, mut constructor: impl FnMut()->T) {
        let cmds = make_commands::<T>(rng, &mut constructor);
        let mut arcs: [Option<ArcShift<T>>; 3] = [();3].map(|_|None);
        debug_println!("Staritng fuzzrun");
        for cmd in cmds {
            debug_println!("=== Applying cmd: {:?} ===", cmd);
            print_arcs(&mut arcs);
            match cmd {
                FuzzerCommand::CreateUpdateArc(chn, val) => {
                    if let Some(arc) = &mut arcs[chn as usize] {
                        arc.upgrade_shared(val);
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
                FuzzerCommand::CloneArc { from, to } => {
                    let clone = arcs[from as usize].clone();
                    arcs[to as usize] = clone;
                }
                FuzzerCommand::DropArc(chn) => {
                    arcs[chn as usize] = None;
                }
            }

        }
        debug_println!("=== No more commands ===");
        print_arcs(&mut arcs);

    }

    fn print_arcs<T:Send+Sync>(arcs: &mut [Option<ArcShift<T>>; 3]) {
        for arc in arcs.iter() {
            if let Some(_arc) = arc {
                //print!("{:? } ", arc.item);
            } else {
                //print!("None, ")
            }
        }
        debug_println!();
        for arc in arcs.iter() {
            if let Some(_arc) = arc {
                //let curr = unsafe { (*arc.item).next.load(atomic::Ordering::SeqCst) };
                //let currcount = if !curr.is_null() { unsafe { (*curr).refcount.load(atomic::Ordering::SeqCst) } } else { 1111 };
                /*debug_println!("{:? } (ctx: {:?}/{}, sc: {}) refs = {}, ", arc.item,
                         unsafe { (*arc.item).shift.current.load(Ordering::AcqRel) },
                         currcount,
                         strongcount,
                         unsafe { (*arc.item).refcount.load(Ordering::AcqRel) });*/
            } else {
                debug_println!("None, ")
            }
        }
    }

    fn make_commands<T:Clone+Eq+Hash+Debug>(rng: &mut StdRng, constructor: &mut impl FnMut()->T) -> Vec<FuzzerCommand<T>> {

        let mut ret = Vec::new();
        for _x in 0..20 {
            match rng.gen_range(0..4) {
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
                _ => unreachable!()
            }
        }
        ret
    }

    #[test]
    fn generic_thread_fuzzing_all() {
        #[cfg(any(loom,miri))]
        const COUNT: u64 = 100;
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
    fn generic_fuzzing_all() {
        #[cfg(any(loom,miri))]
        const COUNT: u64 = 100;
        #[cfg(not(any(loom,miri)))]
        const COUNT: u64 = 10000;
        for i in 0..COUNT {
            model(move||{
                let mut rng = StdRng::seed_from_u64(i);
                let mut counter = 0u32;
                debug_println!("Running seed {}", i);
                run_fuzz(&mut rng, move || -> u32 {
                    counter += 1;
                    counter
                });
            });
        };
    }
}
