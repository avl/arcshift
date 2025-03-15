![build](https://github.com/avl/arcshift/actions/workflows/rust.yml/badge.svg)
![build](https://github.com/avl/arcshift/actions/workflows/shuttle.yml/badge.svg)
![build](https://github.com/avl/arcshift/actions/workflows/loom.yml/badge.svg)
![build](https://github.com/avl/arcshift/actions/workflows/miri.yml/badge.svg)
![build](https://github.com/avl/arcshift/actions/workflows/mutants.yml/badge.svg)
![build](https://github.com/avl/arcshift/actions/workflows/clippy.yml/badge.svg)

# Arcshift

Arcshift is smart pointer type similar to Arc, with the distinction that it allows updating
the value pointed to, with some caveats. Basically, ArcShift is only a good fit if updates
are infrequent, and if it is possible to periodically obtain mutable access to all
ArcShift instances to reload them (freeing up memory).

You can think of ArcShift as an Arc<> over a linked list of versions, with the ability to add 
a new version and automatically reload on read (see [`ArcShift::get`]).

```rust
use std::thread;
use arcshift::ArcShift;

fn main () {
   let mut arc = ArcShift::new("Hello".to_string());
   let mut arc2 = arc.clone();


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
```

## Docs

For docs, see <https://docs.rs/arcshift/> .

The rest of this document is a sort of blogpost about Arcshift. See the docs above for
more user-relevant information!

# Background

I created ArcShift because I wanted to have a low-overhead way to store resources in a computer game project.
The idea is that assets such as 3d-models, textures etc are seldom modified, and putting them in an std::sync::Arc
seems reasonable. 
However, once something is put into an Arc, and that Arc is propagated through the system,
there is no way to modify the value. On solution is to use `Arc<Mutex<T>>`.
However, Mutex access is far from free, even if the mutex is uncontended. If the value is only
rarely updated, paying mutex overhead on each access is undesirable.

### ArcSwap

ArcShift to some extend solves the same problem as ArcSwap (see: <https://docs.rs/arc-swap/> ).

ArcSwap is a container for `Arc<T>`-objects, which allows swapping out the contained
Arc-object without requiring a &mut-reference.

ArcShift, instead, is an `Arc<T>` where the value T can be updated.  It's possible to achieve 
something like this using ArcSwap, by constructing an `Arc<ArcSwap<Arc<T>>>`, but it is not 
as convenient as could be hoped for.

### Mission statement

The requirements for ArcShift are:

 * Regular shared read access should be as fast as for `Arc<T>`, as long as 
   writes have not occurred.
 * Writes can be expensive (but of course not slower than necessary)
 * It is okay if reads become slightly slower after a write has occurred.
 * The implementation should be lock free (so we never have to suspend a thread)
 * The API must be 100% safe and sound.
 * When values are updated, previous values should be dropped as soon as possible. 
 * It should not use thread local variables.
 * It should support Weak handles.
 * It should not have any memory overhead compared to Arc.
 
Regarding the last two points, any type which provides overhead-free access to T will
have to keep a T alive at all time, since otherwise some sort of synchronization (which is expensive)
would be needed before access was granted.

### The Solution Idea

ArcShift is a drop-in replacement for Arc, with the same
general memory layout. I.e, each instance of ArcShift is a pointer to a heap-block containing the actual value.
ArcShift also has a 'next'-pointer. The 'next'-pointer points to
any updated value, and is null before any update has occurred.


#### Diagram of the general idea:

Note, the below diagram is simplified, see text below.
```

 Heap block 1                                  Heap block 2
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓          ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃  refcount: 1                 ┃    ┌─── ➤┃  refcount: 1                ┃
┃  payload: "First version"    ┃    │     ┃  payload: "Second version"  ┃
┃  next_and_state: ptr ────────┃────┘     ┃  next_and_state: nullptr    ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛          ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
                           ⮝
  ArcShift-instance        │
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃  item: ptr ──────────────┘   ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

```

Since each ArcShift-instance contains a pointer to a heap-block containing an instance of the payload type, 'T',
there is no overhead accessing the underlying values, which was one of the design constraints going into the
project. Whenever a value is updated, another node is added to the linked list of heap blocks.

The above diagram is simplified. ArcShift also keeps a 'prev' pointer, pointing at the previous node,
and also contains an 'advance_count', needed to achieve safe reclamation of nodes in the middle of
a longer chain. For unsized types, ArcShift also saves the size in the heap block.

#### Complications

We also want to support ArcShiftWeak instances. These do not guarantee that the heap block immediately
pointed to contains a non-dropped item T. To actually access the payload T, the ArcShiftLight-instance needs
to be upgraded to an ArcShift-instance.

We therefore differ between 'strong' and 'weak' links. The 'strong' links guarantee that the pointed-to
block, and all following blocks, contain valid 'T' values. The 'weak' links only guarantee that the chain
of nodes contains at least one valid T-object. 

When no strong links exist to a node, the payload of the node can be dropped.

#### Correctness

The above description may seem straightforward. However, something this project has taught me is that
lock-free algorithms are no joke. There are thousands of ways various operations can be interleaved.
Between any two lines of code, hundreds of other lines of code could execute.

Arcshift uses copious amounts of unsafe rust code, and lock free algorithms.
These are techniques that are well known to be very hard to get right.
Because of this, Arcshift needed an extensive test-suite.

## Unit tests
Arcshift is verified by a large number of unit tests.

There are a few simple happy-path tests (for example `simple_get`, and others),
and a few different fuzzing test cases. The fuzz cases run randomized combinations
of different operations, on different threads.

There is also an exhaustive set of tests which test all combinations of ArcShift-operations,
on three concurrent threads.

ArcShift is verified using several tools:

 * [Loom](https://github.com/tokio-rs/loom) - A tool to exhaustively try all interleavings of multiple threads.
 * [Shuttle](https://github.com/awslabs/shuttle) - Similar to Loom, but faster and less exhaustive(sort of)
 * [Miri](https://github.com/rust-lang/miri) - A very capable verifier, that is also capable of finding threading errors

[Cargo mutants](https://mutants.rs/) is also used to ensure test coverage.  

## Custom validation features
Enabling the non-default feature 'validate' causes Arcshift to insert canaries in memory,
enabling best-effort detection of wild pointers or threading issues.
The feature 'debug' enables verbose debug-output to stdout.


