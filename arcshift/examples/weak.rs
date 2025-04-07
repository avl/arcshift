use std::cell::RefCell;
use arcshift::{ArcShift, ArcShiftWeak};

fn main() {
    #[allow(dead_code)]
    struct Node {
        parent: Option<ArcShiftWeak<Node>>,
        child: RefCell<Option<ArcShift<Node>>>,
    }

    let mut root = ArcShift::new(Node {
        parent: None,
        child: RefCell::new(None),
    });

    let child = ArcShift::new(Node {
        parent: Some(ArcShift::downgrade(&root)),
        child: RefCell::new(None),
    });

    root.get().child.borrow_mut().replace(child.clone());

    // Both root and child will be dropped here, there will be no memory-leak
}

