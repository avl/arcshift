use std::thread;
use arcshift::ArcShift;

fn main () {
    let mut arc = ArcShift::new("Hello".to_string());
    let mut arc2 = arc.clone();


    let j1 = thread::spawn(move || {
        println!("Value in thread 1: '{}'", arc.get()); //Prints 'Hello'
        arc.update("New value".to_string());
        println!("Updated value in thread 1: '{}'", arc.get()); //Prints 'New value'
    });

    let j2 = thread::spawn(move || {
        // Prints either 'Hello' or 'New value', depending on scheduling:
        println!("Value in thread 2: '{}'", arc2.get());
    });

    j1.join().unwrap();
    j2.join().unwrap();
}
