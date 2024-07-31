# Arcshift

Arcshift is smart pointer type similar to Arc, with the distinction that it allows updating
the value pointed to.

```rust
use std::thread;
use crate::ArcShift;

let mut arc = ArcShift::new("Hello".to_string());
let mut arc2 = arc.clone();


let j1 = thread::spawn(move||{
    println!("Value in thread 1: '{}'", *arc); //Prints 'Hello'
    arc.update("New value".to_string());
    println!("Updated value in thread 1: '{}'", *arc); //Prints 'New value'
});

let j2 = thread::spawn(move||{
    println!("Value in thread 2: '{}'", *arc2); //Prints either 'Hello' or 'New value', depending on scheduling
});

j1.join().unwrap();
j2.join().unwrap();
```

## Docs

For docs, <https://docs.rs/arcshift/> .


## Origin story

I created ArcShift because I wanted to have a low-overhead way to store resources in a computer game project.
The idea is that assets such as 3-models, textures etc are seldom modified, and putting them in an std::sync::Arc
seems reasonable. However, if a Arc<T> is given to a function, there is no way to modify the value T, short of
using Arc<Mutex<T>>. However, Mutex access is far from free, even if the mutex is uncontended.

### ArcSwap

After some googling, I found the crate <https://docs.rs/arc-swap/> . After reading up a bit on its implementation,
I decided that I thought it seemed complicated, and I wasn't convinced that using thread locals to cache
Arc<T> instances for performance was the right choice.

Now, after putting a few weeks into ArcShift, I have a new appreciation for arc-swap. I think it is a superior
choice for most situations. However, if the following conditions are fulfilled, ArcShift may give similar
or better performance while being slightly easier to use:

1) Updates are infrequent
2) Each thread can have its own (mutable) copy of an ArcShift instance

For people familiar with arc-swap, ArcShift-instances behaves much like arc_swap::cache::Cache, while
the ArcShift analog to ArcSwap is ArcShiftLight.

### The Problem

I quite quickly settled on a solution where


