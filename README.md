![build](https://github.com/avl/arcshift/actions/workflows/rust.yml/badge.svg)

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

After some googling, I found the crate <https://docs.rs/arc-swap/> . However, it wasn't exactly what
I was looking for. ArcSwap is a container for Arc<T>-objects, which allows swapping out the contained
Arc-object without requiring a &mut-reference.

What I wanted was simply an Arc<T> where the value T could be updated.

Now, it's possible to achieve something like this using ArcSwap, by constructing an Arc<ArcSwap<Arc<T>>> .

ArcSwap is a fantastic crate, and it is much more mature than ArcShift.
However, for example if the following conditions are fulfilled, ArcShift may give similar
or possibly better performance while being slightly easier to use:

1) Updates are infrequent
2) Each thread can have its own (mutable) copy of an ArcShift instance

For people familiar with arc-swap, ArcShift-instances behaves much like arc_swap::cache::Cache, while
the ArcShift analog to ArcSwap is ArcShiftLight.

### Mission statement

The requirements for ArcShift are:

 * Regular shared read access should be exactly as fast as for Arc<T>
 * Writes can be expensive (but of course not slower than necessary)
 * The implementation should be lock free (so we never have to suspend a thread)
 * The API must be 100% safe and sound.
 * When values are updated, previous values should be dropped when possible 
 * It should be possible to have 'light' variants which do not provide as fast access,
   but on the other hand does not keep older values alive.

Regarding the last two points, any type which provides overhead-free access to T will
have to keep such a T alive at all time, since otherwise some sort of synchronization (which is expensive)
would be needed before access was granted.

### The Solution Idea

I quite quickly settled on a solution where ArcShift is a drop-in replacement for Arc, with the same
general memory layout. I.e, each instance of ArcShift is a pointer to a heap-block containing the actual value.
Just as Arc, ArcShift has a reference count in the heap block. In contrast to Arc, ArcShift only has one refcount,
there's no concept of 'weak' references. ArcShift also has a 'next'-pointer. The 'next'-pointer points to
any updated value, and is null before any update has occurred.


#### Diagram of the general idea:

Note, the below diagram is simplified, see text below.
```

 Heap block 1                                             Heap block 2
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓                     ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃  refcount: 1                   ┃        ┌────────── ➤┃  refcount: 1                   ┃
┃  payload: "First version"      ┃        │            ┃  payload: "Second version"     ┃
┃  next_and_state: ptr ──────────┃────────┘            ┃  next_and_state: nullptr       ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛                     ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
                           ⮝
  ArcShift-instance        │
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃  item: ptr ──────────────┘     ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

```

Since each ArcShift-instance contains a pointer to a heap-block containing an instance of the payload type, 'T',
there is no overhead accessing the underlying values, which was one of the design constraints going into the
project. Whenever a value is updated, another node is added to the linked list of heap blocks.

#### Complications

We also want to support ArcShiftLight instances. These do not guarantee that the heap block immediately
pointed to contains a non-dropped item T. To actually access the payload T, the ArcShiftLight-instance needs
to be upgraded to an ArcShift-instance.

We therefore differ between 'strong' and 'weak' links. The 'strong' links guarantee that the pointed-to
block, and all following blocks, contain valid 'T' values. The 'weak' links only guarantee that the chain
of nodes contains at least one valid T-object. Strong links increase the refcount by 2^19. This means
that at most 2^19 weak links can exist (which is enough for many purposes). With a 64-bit refcount,
the maximum number of strong links is approx 2^64/2^19, about 35000 billion.

Another trick used is that the 2 least significant bits of the 'next_and_state' pointer is a 2-bit integer
with the following interpretation:


| Enum value | Meaning                                                                           |
|------------|-----------------------------------------------------------------------------------|
| 0          | No meaning                                                                        |
| Tentative  | The most recent updated value has not yet been accessed and can be dropped        |
| Superseded | There is a new item that can be used, but the payload of this node is not dropped |
| Dropped    | The payload of this node is dropped                                               |


When no strong links exist to a node, the payload of the node can be dropped.

#### Correctness

The above description may seem straightforward. However, something this project has taught me is that
lock-free algorithms are no joke. There are thousands of ways various operations can be interleaved.
Between any two lines of code, hundreds of other lines of code could execute.

Arcshift uses copious amounts of unsafe rust code, and lock free algorithms.
These are techniques that are well known to be very hard to get right.
Because of this, Arcshift needed an extensive test-suite:

## Unit tests
Arcshift is verified by a large number of unit tests.

There are a few simple happy-path tests (for example `simple_get`, and others),
and a few different fuzzing test cases. The fuzz cases run randomized combinations
of different operations, on different threads.

There is also an exhaustive set of tests which test all combinations of ArcShift-operations,
on three concurrent threads.


## Using 'Loom'
The excellent rust crate 'Loom' is used to verify that there are no race conditions that
can cause failures. See <https://crates.io/crates/loom>.

## Using 'Shuttle'
Shuttle is another test framework for writing multi-threaded code, similar to loom.
See <https://crates.io/crates/shuttle>.

## Using 'Miri'
Miri is used to verify that the unsafe code in Arcshift follows the 'stacked borrows' rules.

## Using built-in validation feature
Enabling the non-default feature 'validate' causes Arcshift to insert canaries in memory,
enabling best-effort detection of wild pointers or threading issues.












