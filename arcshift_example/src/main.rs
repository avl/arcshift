use arcshift::ArcShift;
use std::hint::black_box;
use std::thread;

/// Test
#[inline(never)]
pub fn show_reload(t: &mut ArcShift<u64>) -> &u64 {
    t.get()
}

fn main() {
    let mut arc = ArcShift::new("Hello".to_string());
    let mut arc2 = arc.clone();

    black_box(show_reload(&mut ArcShift::new(black_box(32u64))));
    let j1 = thread::spawn(move || {
        println!("Value in thread 1: '{}'", *arc); //Prints 'Hello'
        arc.update("New value".to_string());
        println!("Updated value in thread 1: '{}'", *arc); //Prints 'New value'
    });

    let j2 = thread::spawn(move || {
        // Prints either 'Hello' or 'New value', depending on scheduling:
        println!("Value in thread 2: '{}'", arc2.get());
    });

    j1.join().unwrap();
    j2.join().unwrap();
}
