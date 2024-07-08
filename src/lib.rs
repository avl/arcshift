use std::cell::UnsafeCell;

pub use std::collections::HashSet;
use std::hint::spin_loop;
use std::process::abort;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, fence, Ordering};

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

pub struct ArcShift<T:'static+Send+Sync> {
    item: *const ItemHolder<T>,
}

unsafe impl<T:'static+Send+Sync> Sync for ArcShift<T> {}

unsafe impl<T:'static+Send+Sync> Send for ArcShift<T> {}

#[repr(align(2))]
struct Dummy {
    _a: u8,
    _b: u8
}


impl<T:Send+Sync> Drop for ArcShift<T> {
    fn drop(&mut self) {
        println!("ArcShift::drop({:?})", self.item);
        drop_item(self.item);
    }
}
#[repr(align(2))]
struct ItemHolder<T:'static+Send+Sync> {
    payload: T,
    next: AtomicPtr<ItemHolder<T>>,
    refcount: atomic::AtomicUsize,
}
impl<T:Send+Sync> Drop for ItemHolder<T> {
    fn drop(&mut self) {
        println!("ItemHolder<T>::drop {:?}", self as *const ItemHolder<T>);
    }
}

struct ArcShiftContext<T:Send+Sync+'static> {
    current: atomic::AtomicPtr<ItemHolder<T>>,
    dummy: bool
}

#[cfg(not(loom))]
fn model(x: impl FnOnce()) {
    x()
}
#[cfg(loom)]
fn model(x: impl Fn()+Send+Sync+'static) {
    loom::model(x)
}

impl<T:'static+Send+Sync> Clone for ArcShift<T> {
    fn clone(&self) -> Self {
        println!("ArcShift::clone({:?})", self.item);
        let cur_item = self.item;
        let rescount = unsafe { (*cur_item).refcount.fetch_add(1, atomic::Ordering::SeqCst) };
        println!("Clone - adding count to {:?}, resulting in count {}", cur_item, rescount + 1);
        ArcShift {
            item: cur_item,
        }
    }
}
impl<T:'static+Send+Sync> ArcShift<T> {

    pub fn new(payload: T) -> ArcShift<T> {
        let item = ItemHolder {
            payload,
            next: AtomicPtr::default(),
            refcount: atomic::AtomicUsize::new(1),
        };
        let cur_ptr = Box::into_raw(Box::new(item));
        ArcShift {
            item: cur_ptr,
        }
    }
    pub fn upgrade(&self, new_payload: T) {
        let item = ItemHolder {
            payload:new_payload,
            next: AtomicPtr::default(),
            refcount: atomic::AtomicUsize::new(1),
        };
        let new_ptr = Box::into_raw(Box::new(item));
        println!("Upgrading {:?} -> {:?} ", self.item, new_ptr);
        let mut candidate = self.item;
        loop {
            match unsafe{&*candidate}.next.compare_exchange(null_mut(), new_ptr, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => {
                    // Upgrade complete.
                    println!("Upgrade complete");
                    fence(Ordering::SeqCst);
                    return;
                }
                Err(other) => {
                    println!("Upgrade not complete yet, spinning");
                    candidate = other;
                }
            }
            spin_loop();
            #[cfg(loom)]
            loom::thread::yield_now();
        }

    }
    pub fn get(&mut self) -> &T {
        println!("Getting {:?}", self.item);
        let cand: *const ItemHolder<T> =  unsafe{&*self.item}.next.load(Ordering::SeqCst) as *const ItemHolder<T>;
        if !cand.is_null() {
            println!("Upgrade to {:?} detected", cand);

            unsafe { (*cand).refcount.fetch_add(1, atomic::Ordering::SeqCst) };
            let count = unsafe{&*self.item}.refcount.fetch_sub(1, Ordering::SeqCst);
            println!("fetch sub received {}", count);
            if count == 1 {
                println!("Actual drop of ItemHolder {:?}", self.item);
                unsafe { (*cand).refcount.fetch_sub(1, atomic::Ordering::SeqCst) };
                _ = unsafe { Box::from_raw(self.item as *mut ItemHolder<T>) };
                // We don't need to increment cand.refcount, since self.item had a count on it, and
                // it is now disappearing. We just don't do self.item.next's decrement, and then we
                // don't need to do the new self.item's increment (since it's the old self.item.next).
            } else {
                println!("No drop of ItemHolder");
            }
            println!("Doing assign");
            self.item = cand;
        }
        println!("Returned payload for {:?}", self.item);
        &unsafe{&*self.item}.payload
    }
}
fn drop_item<T:Send+Sync>(old_ptr: *const ItemHolder<T>) {
    println!("drop_item {:?} about to subtract 1", old_ptr);
    let count = unsafe{&*old_ptr}.refcount.fetch_sub(1, Ordering::SeqCst);
    println!("Drop-item {:?}, post-count = {}", old_ptr, count-1);
    if count == 1 {
        println!("Begin drop of {:?}", old_ptr);
        //let next = *unsafe{&mut *(old_ptr as *mut ItemHolder<T>)}.next.get_mut();
        let next = unsafe {&*old_ptr}.next.load(Ordering::SeqCst);
        if next.is_null() == false {
            println!("Actual drop of ItemHolder {:?} - recursing into {:?}", old_ptr, next);
            drop_item(next);
        }
        println!("Actual drop of ItemHolder {:?}", old_ptr);
        _ = unsafe { Box::from_raw(old_ptr as *mut ItemHolder<T>) };
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


    #[test]
    fn simple_threading2() {

        model(||{
            let shift = ArcShift::new(42u32);
            let mut shift1 = shift.clone();
            let mut shift2 = shift1.clone();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(10_000_000).spawn(move||{

                shift1.upgrade(43);
                println!("t1 dropping");
            }).unwrap();

            let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(10_000_000).spawn(move||{

                std::hint::black_box(shift2.get());
                println!("t2 dropping");

            }).unwrap();
            _=t1.join().unwrap();
            _=t2.join().unwrap();
        });
    }
    #[test]
    fn simple_threading3a() {
        model(|| {
            let shift1 = Arc::new(ArcShift::new(42u32));
            let shift2 = Arc::clone(&shift1);
            let mut shift3 = (*shift1).clone();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(10_000_000).spawn(move || {
                println!(" = On thread t1 =");
                shift1.upgrade(43);
                println!(" = drop t1 =");
            }).unwrap();

            let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(10_000_000).spawn(move || {
                println!(" = On thread t2 =");
                let mut shift = (*shift2).clone();
                std::hint::black_box(shift.get());
                println!(" = drop t2 =");
            }).unwrap();

            let t3 = atomic::thread::Builder::new().name("t3".to_string()).stack_size(10_000_000).spawn(move || {
                println!(" = On thread t3 =");
                std::hint::black_box(shift3.get());
                println!(" = drop t3 =");
            }).unwrap();
            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            _ = t3.join().unwrap();
        });
    }
    #[test]
    fn simple_threading4() {
        model(|| {
            let shift1 = Arc::new(ArcShift::new(42u32));
            let shift2 = Arc::clone(&shift1);
            let shift3 = Arc::clone(&shift1);
            let mut shift4 = (*shift1).clone();
            let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(10_000_000).spawn(move || {
                println!(" = On thread t1 =");
                shift1.upgrade(43);
                println!(" = drop t1 =");
            }).unwrap();

            let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(10_000_000).spawn(move || {
                println!(" = On thread t2 =");
                let mut shift = (*shift2).clone();
                std::hint::black_box(shift.get());
                println!(" = drop t2 =");
            }).unwrap();

            let t3 = atomic::thread::Builder::new().name("t3".to_string()).stack_size(10_000_000).spawn(move || {
                println!(" = On thread t3 =");
                shift3.upgrade(44);
                println!(" = drop t3 =");
            }).unwrap();
            let t4 = atomic::thread::Builder::new().name("t4".to_string()).stack_size(10_000_000).spawn(move || {
                println!(" = On thread t4 =");
                std::hint::black_box(shift4.get());
                println!(" = drop t4 =");
            }).unwrap();
            _ = t1.join().unwrap();
            _ = t2.join().unwrap();
            _ = t3.join().unwrap();
            _ = t4.join().unwrap();
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
    fn run_fuzz<T:Clone+Hash+Eq+'static+Debug+Send+Sync>(rng: &mut StdRng, mut constructor: impl FnMut()->T) {
        let cmds = make_commands::<T>(rng, &mut constructor);
        let mut arcs: [Option<ArcShift<T>>; 3] = [();3].map(|_|None);
        println!("Staritng fuzzrun");
        for cmd in cmds {
            println!("=== Applying cmd: {:?} ===", cmd);
            print_arcs(&mut arcs);
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
        println!("=== No more commands ===");
        print_arcs(&mut arcs);

    }

    fn print_arcs<T:Send+Sync>(arcs: &mut [Option<ArcShift<T>>; 3]) {
        for arc in arcs.iter() {
            if let Some(arc) = arc {
                print!("{:? } ", arc.item);
            } else {
                print!("None, ")
            }
        }
        println!();
        for arc in arcs.iter() {
            if let Some(arc) = arc {
                let curr = unsafe { (*arc.item).next.load(Ordering::SeqCst) };
                let currcount = if !curr.is_null() { unsafe { (*curr).refcount.load(Ordering::Relaxed) } } else { 1111 };
                /*println!("{:? } (ctx: {:?}/{}, sc: {}) refs = {}, ", arc.item,
                         unsafe { (*arc.item).shift.current.load(Ordering::SeqCst) },
                         currcount,
                         strongcount,
                         unsafe { (*arc.item).refcount.load(Ordering::SeqCst) });*/
            } else {
                println!("None, ")
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
        const COUNT: u64 = 100;
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
    #[test]
    fn generic_fuzzing0() {
        let seed = 5;
        model(move||
        {
            let mut rng = StdRng::seed_from_u64(seed);
            let mut counter = 0u32;
            println!("Running seed {}", seed);
            run_fuzz(&mut rng, move || -> u32 {
                counter += 1;
                counter
            });
        });
    }
}
