use std::cell::UnsafeCell;
use std::process::abort;
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::time::Duration;
use rand::{Rng, thread_rng};

pub struct ArcShift<T:'static> {
    item: *const ItemHolder<T>,
    shift: Arc<ArcShiftContext<T>>
}

unsafe impl<T:'static> Send for ArcShift<T> {}

#[repr(align(2))]
struct Dummy {
    _a: u8,
    _b: u8
}

impl<T> Drop for ArcShift<T> {
    fn drop(&mut self) {
        drop_item(self.item)
    }
}
fn is_dummy<T>(ptr: *const ItemHolder<T>) -> bool {
    let as_usize = ptr as usize;
    as_usize & 1 != 0
}
#[repr(align(2))]
struct ItemHolder<T> {
    payload: T,
    refcount: AtomicUsize,
}

struct ArcShiftContext<T:'static> {
    current: AtomicPtr<ItemHolder<T>>,
}

fn make_dummy<T>() -> *mut ItemHolder<T> {
    let t = UnsafeCell::<Dummy>::new(Dummy{_a:0,_b:0});
    let t_ptr = (t.get() as *mut u8).wrapping_offset(1) as *mut ItemHolder<T>; //t_ptr is now guaranteed to point to an odd address. Real ItemHolder instances always have even addresses (because of align(2))
    assert!(is_dummy(t_ptr));
    t_ptr
}
impl<T:'static> Clone for ArcShift<T> {
    fn clone(&self) -> Self {
        let cur_item = self.item;
        unsafe { (*cur_item).refcount.fetch_add(1, Ordering::SeqCst); }
        ArcShift {
            item: cur_item,
            shift: Arc::clone(&self.shift)
        }
    }
}
impl<T:'static> ArcShift<T> {

    pub fn new(payload: T) -> ArcShift<T> {
        let item = ItemHolder {
            payload,
            refcount: AtomicUsize::new(2),
        };
        let cur_ptr = Box::into_raw(Box::new(item));
        ArcShift {
            item: cur_ptr,
            shift: Arc::new(ArcShiftContext {
                current: AtomicPtr::new(cur_ptr),
            })
        }
    }
    pub fn upgrade(&self, new_payload: T) {
        let item = Box::into_raw(Box::new(ItemHolder {
            payload: new_payload,
            refcount: AtomicUsize::new(1),
        }));

        let old_ptr = self.shift.current.swap(item, Ordering::SeqCst);
        drop_item(old_ptr);
    }
    pub fn get(&mut self) -> &T {
        let new_current = self.shift.current.load(Ordering::SeqCst) as *const _;
        if new_current != self.item && is_dummy(new_current) == false {

            let dummy_ptr = make_dummy::<T>();

            let prev_ptr_res = self.shift.current.compare_exchange(new_current as *mut _, dummy_ptr, Ordering::SeqCst, Ordering::SeqCst);
            if let Ok(prev_ptr) = prev_ptr_res {
                if prev_ptr as *const _ == self.item || prev_ptr.is_null()  // Ok, the upgrade is somehow no longer relevant.
                    || is_dummy(prev_ptr) // Someone else is in the process of upgrading
                {
                    // Undo our dummy!
                    _ = self.shift.current.compare_exchange(dummy_ptr, prev_ptr, Ordering::SeqCst, Ordering::SeqCst);
                    //println!("Do we get here?");
                    //abort();

                } else {

                    std::thread::sleep(Duration::from_nanos(rand::thread_rng().gen_range(0..100)));
                    // We now have exclusive ownership of 'current'
                    let actual = unsafe {&*prev_ptr};
                    actual.refcount.fetch_add(1, Ordering::SeqCst);

                    // Give it back for others to enjoy
                    // This will fail if the main upgrade-ptr has changed. But this is okay, we
                    // have a stale pointer, but that could have happened _anyway_ if timing differed a few nanoseconds,
                    // and there isn't (and can't be) any guarantee anyway.
                    if self.shift.current.compare_exchange(dummy_ptr, prev_ptr, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                        _ = actual.refcount.fetch_sub(1, Ordering::SeqCst);
                        //println!("compexch2 failed");
                    }
                    drop_item(self.item);
                    self.item = prev_ptr;
                }
            } else {
                //println!("new_current is stranded");
                //drop_item(new_current);
            }
        }
        assert!(unsafe { (*self.item).refcount.load(Ordering::SeqCst) >= 1 });
        return unsafe { & (*self.item).payload };
    }
}
fn drop_item<T>(old_ptr: *const ItemHolder<T>) {
    if !old_ptr.is_null() && !is_dummy(old_ptr) {
        let old = unsafe{&*old_ptr};

        if old.refcount.fetch_sub(1, Ordering::SeqCst) == 1 {
            _ = unsafe { Box::from_raw(old_ptr as *mut ItemHolder<T>) };
        }

    }
}
impl<T> Drop for ArcShiftContext<T> {
    fn drop(&mut self) {
        loop {
            let old_ptr = self.current.load(Ordering::SeqCst);
            if is_dummy(old_ptr) {
                std::hint::spin_loop();
                continue;
            }
            let dummy = make_dummy();
            let droppable = self.current.compare_exchange(old_ptr, dummy, Ordering::SeqCst, Ordering::SeqCst);
            if let Ok(droppable) = droppable {
                drop_item(old_ptr);
                return;
            }
            std::hint::spin_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;
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
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
    }
    #[test]
    fn simple_upgrade() {
        let mut shift = ArcShift::new(42u32);
        assert_eq!(*shift.get(), 42u32);
        shift.upgrade(43);
        assert_eq!(*shift.get(), 43u32);
    }
    #[test]
    fn simple_threading() {

        let shift = ArcShift::new(42u32);
        let mut shift1 = shift.clone();
        let mut shift2 = shift1.clone();
        let t1 = thread::Builder::new().name("t1".to_string()).spawn(move||{

            for x in 0..1000 {
                if x % 5 == 0 {
                    //println!("A: Upgrade to {}", x);
                    shift1.upgrade(x);
                }
                std::hint::black_box(shift1.get());
                //println!("A: Read {}", shift1.get());
            }
        }).unwrap();

        let t2 = thread::Builder::new().name("t2".to_string()).spawn(move||{

            for x in 0..1000 {
                if x %15 == 0 {
                    //println!("B: Upgrade to {}", x);
                    shift2.upgrade(x);
                }
                std::hint::black_box(shift2.get());
                //println!("B: Read {}", shift2.get());
            }
        }).unwrap();
        _=t1.join().unwrap();
        _=t2.join().unwrap();
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

        let senders = Arc::new(senders);
        let all_possible = Arc::new(all_possible);
        for (threadnr, (cmds, receiver)) in batches.into_iter().zip(receivers).enumerate() {
            let thread_senders = Arc::clone(&senders);
            let thread_all_possible = Arc::clone(&all_possible);
            let jh = Builder::new().name(format!("thread{}", threadnr)).spawn(move||{

                let mut curval : Option<ArcShift<T>> = None;
                for cmd in cmds {
                    //println!("Thread {} doing {:?}", threadnr, cmd);
                    if let Ok(val) = receiver.try_recv() {
                        //println!("Did receive");
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
    fn run_fuzz<T:Clone+Hash+Eq+'static+Debug>(rng: &mut StdRng, mut constructor: impl FnMut()->T, debug: bool) {
        let cmds = make_commands::<T>(rng, &mut constructor);
        if debug {
            //println!("Cmds: {:#?}", cmds);
        }
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
        for x in 0..50 {
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
            let mut rng = StdRng::seed_from_u64(i);
            let mut counter = 0u32;
            println!("--- Seed {} ---", i);
            run_multi_fuzz(&mut rng, move || -> u32 {
                counter += 1;
                counter
            });
        }
    }
    #[test]
    fn generic_fuzzing_all() {
        for i in 0..10000 {
            let mut rng = StdRng::seed_from_u64(i);
            let mut counter = 0u32;
            println!("Running seed {}", i);
            run_fuzz(&mut rng, move || -> u32 {
                counter += 1;
                counter
            }, false);
        }
    }
    #[test]
    fn generic_fuzzing70() {
        {
            let mut rng = StdRng::seed_from_u64(70);
            let mut counter = 0u32;
            println!("Running seed {}", 70);
            run_fuzz(&mut rng, move || -> u32 {
                counter += 1;
                counter
            }, true);
        }
    }
}
