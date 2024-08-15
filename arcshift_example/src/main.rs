use arcshift::ArcShift;
use std::thread;

fn main() {
    let mut arc = ArcShift::new("Hello".to_string());
    let arc2 = arc.clone();

    let j1 = thread::spawn(move || {
        println!("Value in thread 1: '{}'", *arc); //Prints 'Hello'
        arc.update("New value".to_string());
        println!("Updated value in thread 1: '{}'", *arc); //Prints 'New value'
    });

    let j2 = thread::spawn(move || {
        // Prints either 'Hello' or 'New value', depending on scheduling:
        println!("Value in thread 2: '{}'", *arc2);
    });

    j1.join().unwrap();
    j2.join().unwrap();
}
