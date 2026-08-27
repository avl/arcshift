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

#[test]
fn zst_overaligned() {
    // A zero-sized type can still require an alignment greater than 1. The deferred-drop
    // path (`take_boxed_payload`, only compiled without `std`/`nostd_unchecked_panics`)
    // constructs a dangling pointer for the zero-sized allocation; it must be aligned for
    // `T`, otherwise `Box::from_raw` is instant UB. Miri catches a misaligned pointer here.
    #[repr(align(64))]
    struct OverAligned;

    let a = ArcShift::new(OverAligned);
    let mut b = a.clone();
    b.update(OverAligned);
    drop(b);
    drop(a);
}
