use std::mem::MaybeUninit;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};


pub struct Guard<'a,T:'static> {
    item: *const ItemHolder<T>,
    shift: &'a ArcShift<T>
}

pub struct ActualItem<T> {
    payload: T,
    refcount: AtomicUsize,
}
impl<'a,T> Drop for Guard<'a, T> {
    fn drop(&mut self) {
        drop_item(self.item)
    }
}
pub enum ItemHolder<T> {
    Actual(ActualItem<T>),
    Dummy
}
impl<T> ItemHolder<T> {
    pub fn is_dummy(&self) -> bool {
        match self {
            ItemHolder::Actual { .. } => false,
            ItemHolder::Dummy => true
        }
    }
    pub fn expect_actual(&self) -> &ActualItem<T> {
        match self {
            ItemHolder::Actual(a) => {
                a
            }
            ItemHolder::Dummy => {
                // This should absolutely never happen
                std::process::abort()
            }
        }
    }
}

pub struct ArcShift<T:'static> {
    current: AtomicPtr<ItemHolder<T>>,
}


impl<'a,T:'static> Guard<'a,T> {
    pub fn get<'s>(&'s mut self) -> &'s T where 'a:'s {
        let new_current = self.shift.current.load(Ordering::Relaxed) as *const _;
        if new_current != self.item && unsafe { (*new_current).is_dummy() } == false {
            let mut dummy_holder = ItemHolder::Dummy;
            let dummy_ptr = &mut dummy_holder as *mut _;

            let current_ptr = self.shift.current.swap(dummy_ptr, Ordering::SeqCst);

            if current_ptr.is_null()  // Ok, the upgrade is somehow no longer relevant.
                || unsafe { (*current_ptr).is_dummy() } // Someone else is in the process of upgrading
            {
                // Undo our dummy!
                _ = self.shift.current.compare_exchange(dummy_ptr, current_ptr, Ordering::SeqCst, Ordering::SeqCst);
            } else {
                // We now have exclusive ownership of 'current'
                let actual = unsafe {(*current_ptr).expect_actual()};
                unsafe { actual.refcount.fetch_add(1, Ordering::SeqCst); }

                // Give it back for others to enjoy
                // This will fail if the main upgrade-ptr has changed. But this is okay, we
                // have a stale pointer, but that could have happened _anyway_ if timing differed a few nanoseconds,
                // and there isn't (and can't be) any guarantee anyway.
                _ = self.shift.current.compare_exchange(dummy_ptr, current_ptr, Ordering::SeqCst, Ordering::SeqCst);

                drop_item(self.item);
                self.item = current_ptr;
            }
        }

        return unsafe { & (*self.item).expect_actual().payload };
    }
}
fn drop_item<T>(old_ptr: *const ItemHolder<T>) {
    if old_ptr.is_null() == false {
        let old = unsafe{&*old_ptr};
        if let ItemHolder::Actual(actual) = old {
            if actual.refcount.fetch_sub(1, Ordering::Relaxed) == 1 {
                _ = unsafe { Box::from_raw(old_ptr as *mut ItemHolder<T>) };
            }
        }
    }
}
impl<T:'static> ArcShift<T> {
    pub fn new(payload: T) -> ArcShift<T> {
        let item = ItemHolder::Actual(ActualItem {
            payload,
            refcount: AtomicUsize::new(1),
        });
        ArcShift {
            current: AtomicPtr::new(Box::into_raw(Box::new(item))),
        }
    }
    pub fn upgrade(&self, new_payload: T) {
        let item = Box::into_raw(Box::new(ItemHolder::Actual(ActualItem {
            payload: new_payload,
            refcount: AtomicUsize::new(1),
        })));

        let old_ptr = self.current.swap(item, Ordering::Relaxed);
        drop_item(old_ptr);
    }
    pub fn get(&self) -> Guard<T> {
        loop {
            let current = self.current.load(Ordering::Relaxed);
            match unsafe{&*current } {
                ItemHolder::Actual(actual) => {
                    unsafe{actual.refcount.fetch_add(1, Ordering::SeqCst)};
                    return Guard {
                        item: current,
                        shift: self
                    }
                }
                ItemHolder::Dummy => {
                    continue; //Someone is in the midst of upgrading. Wait.
                }
            }
        }
    }
}
impl<T> Drop for ArcShift<T> {
    fn drop(&mut self) {
        let old_ptr = unsafe { self.current.load(Ordering::Relaxed) };
        drop_item(old_ptr);
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use super::*;

    #[test]
    fn simple_get() {
        let shift = ArcShift::new(42u32);

        let mut guard = shift.get();
        assert_eq!(*guard.get(), 42u32);
    }
    #[test]
    fn simple_upgrade() {
        let shift = ArcShift::new(42u32);

        let mut guard = shift.get();
        shift.upgrade(43);
        assert_eq!(*guard.get(), 43u32);
    }
    #[test]
    fn simple_threading() {

        let shift = ArcShift::new(42u32);
        let shift1 = Arc::new(shift);
        let shift2 = Arc::clone(&shift1);
        let t1 = thread::spawn(move||{
            let mut g1 = shift1.get();
            for x in 0..1000 {
                if x % 100 == 0 {
                    println!("A: Upgrade to {}", x);
                    shift1.upgrade(x);
                }
                println!("A: Read {}", g1.get());
            }
        });
        compile_error!("This test finds errors!")
        let t2 = thread::spawn(move||{
            let mut g2 = shift2.get();
            for x in 0..1000 {
                if x %105 == 0 {
                    println!("B: Upgrade to {}", x);
                    shift2.upgrade(x);
                }
                println!("B: Read {}", g2.get());
            }
        });
        _=t1.join();
        _=t2.join();
    }
}
