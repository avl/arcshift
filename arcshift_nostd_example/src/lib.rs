#![no_std]
extern crate alloc;

use arcshift::ArcShift;

pub fn create_arcshift(x: u32) -> ArcShift<u32> {
    ArcShift::new(x)
}

#[cfg(test)]
mod tests {
    use crate::create_arcshift;

    #[test]
    fn it_works() {
        let mut x = create_arcshift(42);
        x.update(32);
        x.reload();
        assert_eq!(*x.get(), 32);
    }
}
