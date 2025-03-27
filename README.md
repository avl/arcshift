![build](https://github.com/avl/arcshift/actions/workflows/rust.yml/badge.svg)
![build](https://github.com/avl/arcshift/actions/workflows/shuttle.yml/badge.svg)
![build](https://github.com/avl/arcshift/actions/workflows/loom.yml/badge.svg)
![build](https://github.com/avl/arcshift/actions/workflows/miri.yml/badge.svg)
![build](https://github.com/avl/arcshift/actions/workflows/clippy.yml/badge.svg)

# ArcShift

ArcShift is smart pointer type similar to Arc, with the distinction that it allows updating
the value pointed to, with some caveats. Basically, ArcShift is only a good fit if updates
are infrequent, and if it is possible to obtain mutable access to ArcShift instances to reload 
them. For the cases where it fits, ArcShift works like faster `Arc<RwLock<T>>`. 

You can think of ArcShift as an Arc<> over a linked list of versions, with the ability to add 
a new version and automatically load the latest value on read (see [`ArcShift::get`]).

```rust
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
```

# Docs

For docs, see <https://docs.rs/arcshift/> .

# Upgrading from 0.2.6

0.2.7 reworks how `shared_get` works. Previously, this method would simply return stale values, in case the
ArcShift instance was stale. With 0.2.7, stale values will be detected, the arcshift will be cloned and reloaded,
and a guard will be returned with a possibly reloaded clone.

This is still pretty fast in the case that the self instance is _not_ stale. However, there is a severe
performance penalty for stale values. Being slow but correct is probably the better default. The method
`non_reloading_get` can be used to get very quick access, while possibly stale.

# Upgrading from 0.1.x

Release 0.2.0 is a breaking change. The major advantage of 0.2.0 is that it has bounded memory consumption at
all times. A value can be updated any number of times, and memory consumption still is only linear in the number
of live ArcShift instances. Payload values only stay in memory if they are actively referenced by at least one
ArcShift instance. This means arcshift no longer requires that all instances are periodically reloaded
in order to avoid excessive memory consumption.

Worst case memory consumption is now:

* Number of unique refs multiplied by `size_of<T>`

plus

* Sum of memory owned by strongly referenced T 

Shared references to ArcShift can no longer provide the most up-to-date value. You should make sure to 
always have owned (mutable) ArcShift objects. The reason for this change is that this is what allows
intermediary values from being dropped quickly.

The API for RCU has been cleaned up. RCU is now always lock-free, and there is no fallible variant.

Please file issues if there is a use case served by 0.1 that cannot be served by 0.2. You are also
welcome to keep using 0.1.

# Background

ArcShift was created as a low-overhead way to store resources in a computer game project.
The idea is that assets such as 3d-models, textures etc are seldom modified, and putting 
them in an std::sync::Arc seems reasonable. 
However, once something is put into an Arc, and that Arc is propagated through the system,
there is no way to modify the value. On solution is to use `Arc<Mutex<T>>`.
However, Mutex access is far from free, even if the mutex is uncontended. If the value is only
rarely updated, paying mutex overhead on each access is undesirable.

### ArcSwap

ArcShift to some extent solves the same problem as ArcSwap (see: <https://docs.rs/arc-swap/> ).

ArcSwap is a container for `Arc<T>`-objects, which allows swapping out the contained
Arc-object without requiring a &mut-reference.

ArcShift, instead, is an `Arc<T>` where the value T can be updated.  It's possible to achieve 
something like this using ArcSwap, by constructing an `Arc<ArcSwap<Arc<T>>>`, but it is not 
as convenient.

### Mission statement

The requirements for ArcShift are:

 * Regular shared read access should be approximately as fast as for `Arc<T>`, as long as 
   writes have not occurred.
 * Writes can be expensive (but of course not slower than necessary)
 * It is okay if reads become slightly slower after a write has occurred.
 * The implementation should be lock free (so we never have to suspend a thread)
 * The API must be 100% safe and sound.
 * When values are updated, previous values should be dropped as soon as possible. 
 * It should not use thread local variables.
 * It should support no_std
 * It should support Weak handles.
 
Regarding the last point, any type which provides overhead-free access to T will
have to keep a T alive at all time, since otherwise some sort of synchronization (which is expensive)
would be needed before access was granted.

### The Solution Idea

ArcShift is a drop-in replacement for Arc, with a slightly larger memory footprint.
I.e, each instance of ArcShift is a pointer to a heap-block containing the actual value.
ArcShift also has a 'next'-pointer, a 'prev'-pointer and an 'advance' counter. The 'next'-pointer points to
any updated value, and is null before any update has occurred.


#### Diagram of the general idea:

Note, the below diagram is simplified, see text below.
```

 Heap block 1                               Heap block 2
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━┓          ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃  weak_count: 2            ┃  ┌───┐   ┃  weak_count: 1             ┃
┃  strong_count: 1          ┃⮜─┘ ┌─┼─ ➤┃  strong_count: 1           ┃
┃  payload: "First version" ┃    │ │   ┃  payload: "Second version" ┃
┃  next: ───────────────────┃────┘ │   ┃  next: null                ┃
┃  prev: null               ┃      └───┃─ prev:                     ┃
┃  advance: 0               ┃          ┃  advance: 0                ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━┛          ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
                        ⮝
  ArcShift-instance     │
┏━━━━━━━━━━━━━━━━━━━━━━━┿━━━┓
┃  item: ptr ───────────┘   ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

```

Since each ArcShift-instance contains a pointer to a heap-block containing an instance of the payload type, 'T',
there is almost no overhead accessing the payload. Whenever a value is updated, another node is added to the 
linked list of heap blocks.

For unsized types, the heap block also contains the object size.

#### Complications

We also want to support ArcShiftWeak instances. These do not guarantee that the heap block immediately
pointed to contains a non-dropped item T. To actually access the payload T, the ArcShiftLight-instance needs
to be upgraded to an ArcShift-instance.

We therefore differ between 'strong' and 'weak' pointers. The 'strong' pointers guarantee that the pointed-to
block, and the latest following block, contain valid 'T' values. The 'weak' pointers only guarantee that the chain
of nodes contains at least one valid T-object. 

When no strong pointers exist to a node, and it is not the latest node, the payload of the node can be dropped.
When the chain contains only weak pointers, the latest node payload can also be dropped.

#### Correctness

Lock-free algorithms are no joke. There are thousands of ways various operations can be interleaved.
Between any two lines of code, hundreds of other lines of code could execute.

ArcShift uses copious amounts of unsafe rust code, and lock free algorithms.
These are techniques that are well known to be very hard to get right.
Because of this, ArcShift has an extensive test-suite.

## Unit tests
ArcShift is verified by a large number of unit tests.

There are a few simple happy-path tests (for example `simple_get`, and others),
and a few different fuzzing test cases. The fuzz cases run randomized combinations
of different operations, on different threads.

There is also an exhaustive set of tests which test all combinations of ArcShift-operations,
on three concurrent threads.

ArcShift is verified using several tools:

 * [Loom](https://github.com/tokio-rs/loom) - A tool to exhaustively try all interleavings of multiple threads.
 * [Shuttle](https://github.com/awslabs/shuttle) - Similar to Loom, but faster and less exhaustive(sort of)
 * [Miri](https://github.com/rust-lang/miri) - A very capable verifier, that is also capable of finding threading errors

[Cargo mutants](https://mutants.rs/) is also used to ensure test coverage, but not run in CI because of
time complexity.

## Custom validation features
Enabling the non-default feature 'validate' causes ArcShift to insert canaries in memory,
enabling best-effort detection of wild pointers or threading issues.
The feature 'debug' enables extremely verbose debug-output to stdout.


