use crate::ArcShift;

#[test]
#[should_panic(expected = "panic: B")]
#[cfg(not(all(miri, feature = "nostd_unchecked_panics")))] // this leaks with nostd_unchecked_panics
fn simple_panic() {
    struct PanicOnDrop(char);
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            if self.0 == 'B' {
                panic!("panic: {}", self.0)
            }
        }
    }
    // Use a box so that T has a heap-allocation, so miri will tell us
    // if it's dropped correctly (it should be)
    let a = ArcShift::new(alloc::boxed::Box::new(PanicOnDrop('A')));
    let mut b = a.clone();
    b.update(alloc::boxed::Box::new(PanicOnDrop('B')));
    drop(b); //This doesn't drop anything, since 'b' is kept alive by next-ref of a
    drop(a); //This will panic, but shouldn't leak memory
}

#[test]
fn smoke_test() {
    let mut x = ArcShift::new(45u64);
    x.update(46);
    x.rcu(|x| *x + 1);
    assert_eq!(*x.get(), 47);
}
