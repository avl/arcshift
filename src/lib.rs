use std::cell::UnsafeCell;

pub use std::collections::HashSet;
use std::hint::spin_loop;
use std::process::abort;
use std::ptr::null_mut;
use std::sync::Arc;
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

unsafe impl<T:'static> Sync for ArcShift<T> {}

unsafe impl<T:'static> Send for ArcShift<T> {}

#[repr(align(2))]
struct Dummy {
    _a: u8,
    _b: u8
}


impl<T> Drop for ArcShift<T> {
    fn drop(&mut self) {
        println!("ArcShift drop called {:?}", self.item);
        let old_ptr = self.item;
        if !old_ptr.is_null() && !is_dummy(old_ptr) {
            loop {
                //let old = unsafe{&*old_ptr};
                let count = unsafe{&*old_ptr}.refcount.load(Ordering::SeqCst);

                println!("Arcshift count: {} @ {:?}", count, self.item);
                if count == 0 {
                    println!("Internal error");
                    abort();
                }
                if count == 1 {
                    println!("ArcShift drop count = 1 {:?}", self.item);
                    // If count has reached 1, it can't increase, because we're the only
                    // ones holding it.
                    // In principle, we could use non-atomic ops here.
                    drop_item(old_ptr);
                    return;
                }
                if count == 2 {
                    println!("ArcShift drop count = 2 {:?}", self.item);
                    let dummy= make_dummy();
                    assert!(!is_dummy(old_ptr));
                    match unsafe{&*old_ptr}.shift.current.compare_exchange(old_ptr as *mut _, dummy, Ordering::SeqCst, Ordering::SeqCst) {
                        Ok(_) => {
                            println!("ArcShift drop count =2 {:?} injected dummy", self.item);
                            match unsafe{&*old_ptr}.refcount.compare_exchange(2, 1, Ordering::SeqCst,Ordering::SeqCst) {
                                Ok(_) => {
                                    // What is known:
                                    // * self.item has two owners
                                    // * One of them is 'self'
                                    // * The other is 'context'
                                    // * The context may have other owners
                                    // * The number of context owners can increase
                                    // * No-one else has access to old_ptr, or can gain such access
                                    // * We own the Arc inside ItemHolder.

                                    println!("ArcShift drop count =2 {:?} injected dummy {:?} - no race", self.item, dummy);
                                    let item_mut = unsafe { &mut *(self.item as *mut ItemHolder<T>) };
                                    let the_arc = std::mem::replace(&mut item_mut.shift, atomic::Arc::new(
                                        ArcShiftContext {
                                            current: atomic::AtomicPtr::default(),
                                            dummy: true
                                        })
                                    );

                                    let mut context_has_our_ptr;
                                    match the_arc.current.compare_exchange(dummy, old_ptr as *mut _, Ordering::SeqCst, Ordering::SeqCst) {
                                        Ok(_) => {
                                            println!("DUmmy {:?} was returned for {:?}", dummy, old_ptr);
                                            context_has_our_ptr = true;
                                            // Ok
                                        }
                                        Err(replaced_by) => {
                                            // Dummy was replaced, presumably by upgrade. This means that ItemHolder is definitely unused now, after all
                                            println!("Dummy was replaced by {:?}, presumably by upgrade", replaced_by);
                                            context_has_our_ptr = false;
                                            spin_loop();
                                        }
                                    }

                                    if let Ok(mut inner) = atomic::Arc::<ArcShiftContext<T>>::try_unwrap(the_arc) { //TODO: BAD!
                                        // If we get here, there are now no more owners of neither the item or the context.
                                        println!("There were no more owners of the Arc");
                                        if context_has_our_ptr {
                                            inner.current = atomic::AtomicPtr::<ItemHolder<T>>::default();
                                        }
                                        println!("Actual drop(2) of ItemHolder");
                                        let _item = unsafe { Box::from_raw(item_mut) };
                                        return; //Done
                                    } else {
                                        println!("There were other owners of the Arc!");
                                    }
                                    return;
                                }
                                Err(_) => {
                                    match unsafe{&*old_ptr}.shift.current.compare_exchange(dummy, old_ptr as *mut _, Ordering::SeqCst, Ordering::SeqCst) {
                                        Ok(_) => {
                                            println!("ArcShift drop count =2 {:?} injected dummy - there was a race", self.item);
                                            spin_loop();
                                            continue;
                                        }
                                        Err(_) => {
                                            // No one should ever replace a dummy
                                            eprintln!("Internal error - unreachable code reached.");
                                            abort();
                                        }
                                    }
                                }
                            }

                        }
                        Err(_) => {
                            println!("ArcShift drop - context doesn't have our item");
                            // 'item' is no longer 'current', and thus never will be again
                            drop_item(old_ptr);
                            return;
                        }
                    }
                }
                if count >=3 {
                    println!("ArcShift drop count >= 3 {:?}", self.item);
                    match unsafe{&*old_ptr}.refcount.compare_exchange(count, count - 1,Ordering::SeqCst,Ordering::SeqCst) {
                        Ok(_) => {
                            println!("ArcShift drop count >= 3 {:?} sucecss", self.item);
                            //We successfully reduced count by one.
                            return;
                        }
                        Err(_) => {
                            println!("ArcShift drop count >= 3 {:?} raced", self.item);
                            // Someone else intercepted, we need to try again (we can't just
                            // decrease the count by 1, since we could then end up at count = '1',
                            // in which case we wouldn't know if only the 'context' remained,
                            // or if someone else owns it, and in either case we can no longer
                            // interact with the object soundly to determine which case is actual.
                            spin_loop();
                            continue;
                        }
                    }
                }
            }


        }
        println!("Exiting ArcShiftDrop");
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
        println!("ItemHolder<T>::drop {:?} (strongcount: {})", self as *const ItemHolder<T>, atomic::Arc::strong_count(&self.shift));
    }
}

struct ArcShiftContext<T:'static> {
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

fn make_dummy<T>() -> *mut ItemHolder<T> {
    let t = UnsafeCell::<Dummy>::new(Dummy{_a:0,_b:0});
    let t_ptr = (t.get() as *mut u8).wrapping_offset(1) as *mut ItemHolder<T>; //t_ptr is now guaranteed to point to an odd address. Real ItemHolder instances always have even addresses (because of align(2))
    t_ptr
}
impl<T:'static> Clone for ArcShift<T> {
    fn clone(&self) -> Self {
        let cur_item = self.item;
        let rescount = unsafe { (*cur_item).refcount.fetch_add(1, atomic::Ordering::Relaxed) };
        println!("Clone - adding count to {:?}, resulting in count {}", cur_item, rescount + 1);
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
                current: atomic::AtomicPtr::<ItemHolder<T>>::default(), //null
                dummy: false,
            }),
            refcount: atomic::AtomicUsize::new(2),
        };
        let cur_ptr = Box::into_raw(Box::new(item));

        let ashift_instance = unsafe { atomic::Arc::get_mut(&mut (*cur_ptr).shift).unwrap() };
        println!("New primary ItemHolder {:?}, with context = C({:?})", cur_ptr, ashift_instance as *const ArcShiftContext<T>);

        unsafe { *ashift_instance = ArcShiftContext {
            current: {
                atomic::AtomicPtr::<ItemHolder<T>>::new(cur_ptr)
                },
            dummy: false
            }
        };
        ArcShift {
            item: cur_ptr,
        }
    }
    pub fn upgrade(&self, new_payload: T) {
        println!(" = = = upgrade = = =");
        let old_shift = unsafe{ &(*self.item).shift };

        assert!(!old_shift.dummy);

        let item = Box::into_raw(Box::new(ItemHolder {
            payload: new_payload,
            shift: atomic::Arc::new(ArcShiftContext{current: atomic::AtomicPtr::default(), dummy: true}),
            refcount: atomic::AtomicUsize::new(1),
        }));
        println!("New upgraded ItemHolder {:?}", item);

        loop {
            let old_current = old_shift.current.load(Ordering::SeqCst);
            if (old_current.is_null() || is_dummy(old_current))
            {
                // TODO: It might never become non-null. Fix this!
                println!("is null or dummy");
            }
            else {
                println!("Doing upgrade switch");
                if let Ok(_) = old_shift.current.compare_exchange(old_current, item, atomic::Ordering::AcqRel, atomic::Ordering::Relaxed) {
                    println!("Upgrade calling drop on {:?}, old shift strongcount: {}", old_current, atomic::Arc::strong_count(&old_shift));
                    drop_item(old_current);
                    return;
                }
            }
            println!("Upgrade raced");
            #[cfg(loom)]
            loom::thread::yield_now();
            spin_loop();
            continue;
        }
    }
    pub fn get(&mut self) -> &T {
        println!(" = = = get = = =");
        let cur_shift = unsafe { &(*self.item).shift };
        let new_current = cur_shift.current.load(atomic::Ordering::Relaxed) as *const _;
        if new_current != self.item && is_dummy(new_current) == false && new_current.is_null() == false {

            let dummy_ptr = make_dummy::<T>();

            let prev_ptr_res = cur_shift.current.compare_exchange(new_current as *mut _, dummy_ptr, atomic::Ordering::Acquire, atomic::Ordering::Relaxed);
            if let Ok(prev_ptr) = prev_ptr_res {
                if prev_ptr as *const _ == self.item || prev_ptr.is_null()  // Ok, the upgrade is somehow no longer relevant.
                    || is_dummy(prev_ptr) // Someone else is in the process of upgrading
                {
                    // Undo our dummy!
                    _ = cur_shift.current.compare_exchange(dummy_ptr, prev_ptr, atomic::Ordering::Release, atomic::Ordering::Relaxed);
                    println!("Undoing dummy {:?} to {:?}", dummy_ptr, prev_ptr);

                } else {

                    // We now have exclusive ownership of 'current'
                    let actual = unsafe {&*prev_ptr};
                    let prevcount = actual.refcount.fetch_add(1, atomic::Ordering::Relaxed);
                    println!("Adding count to {:?}, giving count = {}", prev_ptr, prevcount + 1);
                    if (&*actual.shift) as *const ArcShiftContext<T> != (&**cur_shift) as *const ArcShiftContext<T> {
                        let actual_mut = unsafe {&mut *prev_ptr};
                        actual_mut.shift = atomic::Arc::clone(cur_shift);
                    }
                    let actual = unsafe {&*prev_ptr};

                    // Give it back for others to enjoy
                    // This will fail if the main upgrade-ptr has changed. But this is okay, we
                    // have a stale pointer, but that could have happened _anyway_ if timing differed a few nanoseconds,
                    // and there isn't (and can't be) any guarantee anyway.
                    if cur_shift.current.compare_exchange(dummy_ptr, prev_ptr, atomic::Ordering::Release, atomic::Ordering::Relaxed).is_err() {
                        println!("Subtracting on dummy-hand back {:?}", prev_ptr);
                        _ = actual.refcount.fetch_sub(1, atomic::Ordering::Relaxed);
                    }
                    println!("ArcShift::get() drop_item: {:?} (cont {:?}) (dummy = {:?})", self.item, prev_ptr, dummy_ptr);
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
        println!("ItemHolder dropper: {} @{:?} (shift stroungcount: {})", counter, old_ptr, atomic::Arc::strong_count(&old.shift));
        if counter == 1 {
            println!("Actual drop of ItemHolder {:?}", old_ptr);
            let temp = unsafe { Box::from_raw(old_ptr as *mut ItemHolder<T>) };
            println!("Arc strongcount: {}", atomic::Arc::strong_count(&temp.shift));
        }

    }
}
impl<T> Drop for ArcShiftContext<T> {
    fn drop(&mut self) {
        println!("Starting ArcShiftContext C({:?}) loop, dummy = {:?}", self as *const ArcShiftContext<T>, self.dummy);
        if self.dummy {
            assert!(self.current.load(Ordering::SeqCst).is_null());
        }
        loop
        {
            let old_ptr = self.current.load(atomic::Ordering::Relaxed);
            println!("Arcshiftcontext drop: {:?}", old_ptr);
            if old_ptr.is_null() {
                return; //Nothing to do
            }
            println!("Arcshiftcontext contains non-null");
            if is_dummy(old_ptr) {
                println!("Arcshiftcontext contains dummy at drop-time!");
                abort();
                #[cfg(loom)]
                loom::thread::yield_now();
                std::hint::spin_loop();
            }
            compile_error!("It may now be working!!")
            let dummy = make_dummy();
            let droppable = self.current.compare_exchange(old_ptr, dummy, atomic::Ordering::SeqCst, atomic::Ordering::SeqCst);

            if let Ok(_) = droppable {
                println!("Arcshift dropper calling drop_item: {:?}", old_ptr);
                drop_item(old_ptr);
                return;
            }
            println!("Arcshiftcontext current was modified while dropping!");
            continue;
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

    fn do_simple_threading2() {
        let shift1 = Arc::new(ArcShift::new(42u32));
        let shift2 = Arc::clone(&shift1);
        let shift3 = Arc::clone(&shift1);
        let t1 = atomic::thread::Builder::new().name("t1".to_string()).stack_size(10_000_000).spawn(move||{
            println!(" = On thread t1 =");
            shift1.upgrade(43);
            println!(" = drop t1 =");
        }).unwrap();

        let t2 = atomic::thread::Builder::new().name("t2".to_string()).stack_size(10_000_000).spawn(move||{
            println!(" = On thread t2 =");
            let mut shift = (*shift2).clone();
            //std::hint::black_box(shift.get());
            println!(" = drop t2 =");
        }).unwrap();

        let t3 = atomic::thread::Builder::new().name("t3".to_string()).stack_size(10_000_000).spawn(move||{
            println!(" = On thread t3 =");
            shift3.upgrade(44);
            println!(" = drop t3 =");
        }).unwrap();
        _=t1.join().unwrap();
        _=t2.join().unwrap();
        _=t3.join().unwrap();
    }

    #[test]
    fn simple_threading2() {
        model(||{
            println!("=== Starting run ===");
            do_simple_threading2();
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

    fn print_arcs<T>(arcs: &mut [Option<ArcShift<T>>; 3]) {
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
                let curr = unsafe { (*arc.item).shift.current.load(Ordering::SeqCst) };
                let currcount = if !curr.is_null() && !is_dummy(curr) { unsafe { (*curr).refcount.load(Ordering::Relaxed) } } else { 1111 };
                let strongcount = if !curr.is_null() && !is_dummy(curr) { unsafe { atomic::Arc::strong_count(&(*curr).shift) } } else { 1111 };
                println!("{:? } (ctx: {:?}/{}, sc: {}) refs = {}, ", arc.item,
                         unsafe { (*arc.item).shift.current.load(Ordering::SeqCst) },
                         currcount,
                         strongcount,
                         unsafe { (*arc.item).refcount.load(Ordering::SeqCst) });
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
