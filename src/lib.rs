use std::cell::UnsafeCell;

pub use std::collections::HashSet;
use std::sync::atomic::{AtomicPtr, Ordering};

#[cfg(not(loom))]
mod atomic {
    pub use std::sync::Arc;
    pub use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
    pub use std::thread;

}

#[cfg(loom)]
mod atomic {
    pub use loom::sync::Arc;
    pub use loom::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
    pub use loom::thread;
}
use std::time::Duration;
use rand::{Rng};

pub struct ArcShift<T:'static> {
    item: *const ItemHolder<T>,
}

unsafe impl<T:'static> Send for ArcShift<T> {}

#[repr(align(2))]
struct Dummy {
    _a: u8,
    _b: u8
}

impl<T> Drop for ArcShift<T> {
    fn drop(&mut self) {
        println!("ArcShift drop called");
        let old_ptr = self.item;
        if !old_ptr.is_null() && !is_dummy(old_ptr) {
            let old = unsafe{&*old_ptr};
            let count = old.refcount.fetch_sub(1, atomic::Ordering::AcqRel);
            println!("Arcshift drop refcount: {}", count);
            if count == 1 {
                // Simple case, we're definitely the last retainer of the ItemHolder,
                // not even its ShiftContext points to this ItemHolder. It must just be dropped.
                println!("Dropping ItemHolder");
                _ = unsafe { Box::from_raw(old_ptr as *mut ItemHolder<T>) }; //#2
            } else if count == 2 {
                println!("ArcShift drop X/Y case");
                // There are two possibilities:
                //
                // X: We're the last retainer of the ItemHolder. The other count is just from
                // the context (circular reference here). If this is the case, no-one else
                // can access the ItemHolder, and we can leisurely check that the contained 'current'
                // is in fact identical to our 'item'. In this case, we must drop the ItemHolder.
                //
                // Y: We are not the last retainer of the ItemHolder. Some other ArcShift somewhere
                // also has a reference to it. In this case, 'current' will not be equal to 'item'.
                // In this case, we should not drop ItemHolder - it will be done by that other AccShift-

                // Regardless, As one of the counts is our reference, we know ItemHolder will not be dropped.

                if old.shift.current.load(Ordering::Relaxed) as *const _ == old_ptr { //#3
                    // Since this load, and the one of refcount, are not done atomically,
                    // at this point we only know that 'current' is equal to 'item'.
                    // This has to be because we're the only retainer of the ItemHolder.
                    //
                    // Proof:
                    // Let's call the current thread A.
                    // If we're in case 'X' above, things are simple. We should just drop ItemHolder.
                    // If we're in case 'Y' above, things are more tricky.
                    // In this case, some other thread B must have had the ItemHolder at point #3,
                    // but then subsequently have given it back to the ShiftContext. However,
                    // the only transition that puts a pointer in the shiftcontext is the 'upgrade'-operation,
                    // and it never reuses pointers which still exist. So it couldn't have used our ItemHolder.
                    // This means that we have a contradiction, and if we get here, we must be in case X.

                    if let Ok(_) = old.refcount.compare_exchange(2, 0, atomic::Ordering::AcqRel, Ordering::Relaxed) {
                        println!("Probably Dropping ItemHolder X-arc {:?}", old_ptr);
                        _ = unsafe { Box::from_raw(old_ptr as *mut ItemHolder<T>) }; //#2
                    } else {
                        println!("Raced, not dropping");
                    }
                } else {
                    println!("Not dropping ItemHolder Y");
                    // The 'current' does not contain 'item'. There is no transition that
                    // can ever restore 'current' to 'item', so we know the ShiftContext can't own 'item'.
                    // It's safe to do nothing.
                }

            }

        }
    }
}
fn is_dummy<T>(ptr: *const ItemHolder<T>) -> bool {
    let as_usize = ptr as usize;
    as_usize & 1 != 0
}
#[repr(align(2))]
struct ItemHolder<T:'static> {
    payload: T,
    shift: atomic::Arc<ArcShiftContext<T>>,
    refcount: atomic::AtomicUsize,
}
impl<T> Drop for ItemHolder<T> {
    fn drop(&mut self) {
        println!("ItemHolder<T>::drop {:?}", self as *const ItemHolder<T>);
    }
}

struct ArcShiftContext<T:'static> {
    current: atomic::AtomicPtr<ItemHolder<T>>,
}

#[cfg(not(loom))]
fn model(x: impl FnOnce()) {
    x()
}
#[cfg(loom)]
fn model(x: impl Fn()+Send+Sync+'static) {
    loom::model(x)
}

fn make_dummy<T>() -> *mut ItemHolder<T> {
    let t = UnsafeCell::<Dummy>::new(Dummy{_a:0,_b:0});
    let t_ptr = (t.get() as *mut u8).wrapping_offset(1) as *mut ItemHolder<T>; //t_ptr is now guaranteed to point to an odd address. Real ItemHolder instances always have even addresses (because of align(2))
    t_ptr
}
impl<T:'static> Clone for ArcShift<T> {
    fn clone(&self) -> Self {
        let cur_item = self.item;
        unsafe { (*cur_item).refcount.fetch_add(1, atomic::Ordering::Relaxed); }
        ArcShift {
            item: cur_item,
        }
    }
}
impl<T:'static> ArcShift<T> {

    pub fn new(payload: T) -> ArcShift<T> {
        let item = ItemHolder {
            payload,
            shift: atomic::Arc::new(ArcShiftContext {
                current: atomic::AtomicPtr::<ItemHolder<T>>::default() //null
            }),
            refcount: atomic::AtomicUsize::new(2),
        };
        let cur_ptr = Box::into_raw(Box::new(item));
        println!("New primary ItemHolder {:?}", cur_ptr);

        unsafe { *atomic::Arc::get_mut(&mut (*cur_ptr).shift).unwrap() = ArcShiftContext {
            current: atomic::AtomicPtr::<ItemHolder<T>>::new(cur_ptr)
            }
        };
        ArcShift {
            item: cur_ptr,
        }
    }
    pub fn upgrade(&self, new_payload: T) {
        let old_shift = unsafe{ &(*self.item).shift };
        let shift = atomic::Arc::clone(old_shift);
        let item = Box::into_raw(Box::new(ItemHolder {
            payload: new_payload,
            shift,
            refcount: atomic::AtomicUsize::new(1),
        }));
        println!("New upgraded ItemHolder {:?}", item);

        let old_ptr = old_shift.current.swap(item, atomic::Ordering::AcqRel);
        println!("Upgrade calling drop on {:?}", old_ptr);
        drop_item(old_ptr);
    }
    pub fn get(&mut self) -> &T {
        let cur_shift = unsafe { &(*self.item).shift };
        let new_current = cur_shift.current.load(atomic::Ordering::Relaxed) as *const _;
        if new_current != self.item && is_dummy(new_current) == false {

            let dummy_ptr = make_dummy::<T>();

            let prev_ptr_res = cur_shift.current.compare_exchange(new_current as *mut _, dummy_ptr, atomic::Ordering::Acquire, atomic::Ordering::Relaxed);
            if let Ok(prev_ptr) = prev_ptr_res {
                if prev_ptr as *const _ == self.item || prev_ptr.is_null()  // Ok, the upgrade is somehow no longer relevant.
                    || is_dummy(prev_ptr) // Someone else is in the process of upgrading
                {
                    // Undo our dummy!
                    _ = cur_shift.current.compare_exchange(dummy_ptr, prev_ptr, atomic::Ordering::Release, atomic::Ordering::Relaxed);

                } else {

                    // We now have exclusive ownership of 'current'
                    let actual = unsafe {&*prev_ptr};
                    actual.refcount.fetch_add(1, atomic::Ordering::Relaxed);

                    // Give it back for others to enjoy
                    // This will fail if the main upgrade-ptr has changed. But this is okay, we
                    // have a stale pointer, but that could have happened _anyway_ if timing differed a few nanoseconds,
                    // and there isn't (and can't be) any guarantee anyway.
                    if cur_shift.current.compare_exchange(dummy_ptr, prev_ptr, atomic::Ordering::Release, atomic::Ordering::Relaxed).is_err() {
                        _ = actual.refcount.fetch_sub(1, atomic::Ordering::Relaxed);
                    }
                    println!("ArcShift::get() drop_item");
                    drop_item(self.item);
                    self.item = prev_ptr;
                }
            }
        }

        return unsafe { & (*self.item).payload };
    }
}
fn drop_item<T>(old_ptr: *const ItemHolder<T>) {
    if !old_ptr.is_null() && !is_dummy(old_ptr) {
        let old = unsafe{&*old_ptr};

        let counter = old.refcount.fetch_sub(1, atomic::Ordering::AcqRel);
        println!("ItemHolder dropper: {}", counter);
        if counter == 1 {
            println!("Dropping ItemHolder {:?}", old_ptr);
            _ = unsafe { Box::from_raw(old_ptr as *mut ItemHolder<T>) };
        }

    }
}
impl<T> Drop for ArcShiftContext<T> {
    fn drop(&mut self) {
        println!("Arcshiftcontext drop");
        loop {
            let old_ptr = self.current.load(atomic::Ordering::Relaxed);
            if is_dummy(old_ptr) {
                #[cfg(loom)]
                loom::thread::yield_now();
                std::hint::spin_loop();
                continue;
            }
            let dummy = make_dummy();
            let droppable = self.current.compare_exchange(old_ptr, dummy, atomic::Ordering::Relaxed, atomic::Ordering::Relaxed);
            if let Ok(_) = droppable {
                println!("Arcshift dropper calling drop_item");
                drop_item(old_ptr);
                return;
            }
            #[cfg(loom)]
            loom::thread::yield_now();
            std::hint::spin_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fmt::Debug;
    use std::hash::Hash;

    use std::io::Chain;
    use std::rc::Rc;
    use std::sync::RwLock;
    use std::thread;
    use std::thread::Builder;
    use crossbeam_channel::bounded;
    use super::*;

    use rand::{Rng, SeedableRng, thread_rng};
    use rand::prelude::StdRng;


    #[test]
    fn simple_get() {
        model(|| {
            let mut shift = ArcShift::new(42u32);
            assert_eq!(*shift.get(), 42u32);
        })
    }
    #[test]
    fn simple_upgrade() {
        model(||{
            let mut shift = ArcShift::new(42u32);
            assert_eq!(*shift.get(), 42u32);
            shift.upgrade(43);
            assert_eq!(*shift.get(), 43u32);
        });
    }

    fn do_simple_threading() {
        let shift = ArcShift::new(42u32);
        let mut shift1 = shift.clone();
        let mut shift2 = shift1.clone();
        let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(10_000_000).spawn(move||{

            for x in 0..2 {
                if x == 0 {
                    shift1.upgrade(x);
                }
                std::hint::black_box(shift1.get());
                std::hint::black_box(shift1.clone());
            }
        }).unwrap();

        let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(10_000_000).spawn(move||{

            for x in 0..2 {
                if x == 1 {
                    shift2.upgrade(x);
                }
                std::hint::black_box(shift2.get());
                std::hint::black_box(shift2.clone());
            }
        }).unwrap();
        _=t1.join().unwrap();
        _=t2.join().unwrap();
    }
    #[test]
    fn simple_threading() {


        model(||{
            do_simple_threading();
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

        let senders = atomic::Arc::new(senders);
        let all_possible : atomic::Arc<HashSet<T>>= atomic::Arc::new(all_possible);
        for (threadnr, (cmds, receiver)) in batches.into_iter().zip(receivers).enumerate() {
            let thread_senders = atomic::Arc::clone(&senders);
            let thread_all_possible:atomic::Arc<HashSet<T>> = atomic::Arc::clone(&all_possible);
            let jh = atomic::thread::Builder::new().name(format!("thread{}", threadnr)).spawn(move||{

                let mut curval : Option<ArcShift<T>> = None;
                for cmd in cmds {
                    if let Ok(val) = receiver.try_recv() {
                        curval = Some(val);
                    }
                    match cmd {
                        FuzzerCommand::CreateUpdateArc(_, val) => {
                            if let Some(curval) = &mut curval {
                                curval.upgrade(val);
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
                        FuzzerCommand::CloneArc { from:_, to } => {
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
    fn run_fuzz<T:Clone+Hash+Eq+'static+Debug>(rng: &mut StdRng, mut constructor: impl FnMut()->T) {
        let cmds = make_commands::<T>(rng, &mut constructor);
        let mut arcs: [Option<ArcShift<T>>; 3] = [();3].map(|_|None);

        for cmd in cmds {
            match cmd {
                FuzzerCommand::CreateUpdateArc(chn, val) => {
                    if let Some(arc) = &mut arcs[chn as usize] {
                        arc.upgrade(val);
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
    }

    struct ExpectedItem<T> {
        primary: HashSet<T>,
        extra: Vec<Rc<RefCell<ExpectedItem<T>>>>
    }
    impl<T:Clone+Hash+Eq> ExpectedItem<T> {
        fn new() -> ExpectedItem<T> {
            ExpectedItem {
                primary:HashSet::new(),
                extra: vec![]
            }
        }
        fn getall_impl(&self, ret: &mut HashSet<T>, recstop: &mut HashSet<*const ExpectedItem<T>>) {
            if !recstop.insert(self as *const _) {
                return;
            }
            for item in self.primary.iter().cloned() {
                ret.insert(item);
            }
            for item in self.extra.iter() {
                item.borrow().getall_impl(ret, recstop);
            }
        }
        fn getall(&self) -> HashSet<T> {
            let mut temp = HashSet::new();
            self.getall_impl(&mut temp, &mut HashSet::new());
            temp
        }
    }
    fn make_commands<T:Clone+Eq+Hash+Debug>(rng: &mut StdRng, constructor: &mut impl FnMut()->T) -> Vec<FuzzerCommand<T>> {

        let mut ret = Vec::new();
        for _x in 0..50 {
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
        for i in 0..10000 {
            model(move||{

                let mut rng = StdRng::seed_from_u64(i);
                let mut counter = 0u32;
                println!("--- Seed {} ---", i);
                run_multi_fuzz(&mut rng, move || -> u32 {
                    counter += 1;
                    counter
                });
            });
        }
    }
    #[test]
    fn generic_fuzzing_all() {
        const COUNT: u64 = 10000;
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
    fn generic_fuzzing70() {
        model(||
        {
            let mut rng = StdRng::seed_from_u64(70);
            let mut counter = 0u32;
            println!("Running seed {}", 70);
            run_fuzz(&mut rng, move || -> u32 {
                counter += 1;
                counter
            });
        });
    }
}
