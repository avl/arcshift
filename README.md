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
Because of this, Arcshift needed an extensive test-suite.

## Unit tests
Arcshift is verified by a large number of unit tests.

There are a few simple happy-path tests (for example `simple_get`, and others),
and a few different fuzzing test cases. The fuzz cases run randomized combinations
of different operations, on different threads.

There is also an exhaustive set of tests which test all combinations of ArcShift-operations,
on three concurrent threads.


## Using built-in validation feature
Enabling the non-default feature 'validate' causes Arcshift to insert canaries in memory,
enabling best-effort detection of wild pointers or threading issues.


# Lessons learned

While developing Arcshift, I've learned a few lessons.

## Lock-free algorithms really are hard

It is well known that lock free algorithms are difficult to get right. However,
after personally stepping on a few rakes, this truth has become more real to me.

### Lock-free bug example #1
One bug I had, which in hindsight is extremely obvious, is (broken) code I had, which
looked a bit like this:

```rust
    
let count = get_refcount(item)
        .fetch_sub(STRONG_COUNT, Ordering::SeqCst);
if count == STRONG_COUNT {
    drop_item(item);
} else if count < 2*STRONG_COUNT { //WRONG!
    drop_payload(item);
}

```

The idea is that we have a strong refcount to the node 'item'. We subtract this refcount.
If the previous refcount value (returned by AtomicUsize::fetch_sub) is STRONG_COUNT, that means our 
reference was the only remaining reference, and we can now drop 'item'. This works.

The next condition, tries to check if the previous refcount was < 2 * STRONG_COUNT. If so,
this should mean that there were no other strong counts to the node, and we can thus drop
the payload of 'item' (but not the entire node).

However, this does absolutely not work!

After the line `.fetch_sub(STRONG_COUNT, Ordering::SeqCst);`, absolutely nothing is known about
'item'! Some other thread could easily have reduced the count and dropped 'item' before we
can execute `drop_payload(item)`. 

This is in fact a pattern that is worth remembering. After reducing a refcount on a reference counted
pointer, you do no longer have access to the pointee.

### Lock-free bug example #2

As detailed in the documentation, Arcshift uses a single 64 bit atomic counter to contain
both a 19-bit weak refcount (for ArcShiftLight-instances) and a 45-bit strong refcount 
(for ArcShift-instances).

This poses an interesting challenge, when increasing the count. Say we increase the weak count
like this:

```rust
let count = item.fetch_add(1, Ordering::SeqCst);
if count == MAX_WEAK_COUNT { //WRONG!
    item.fetch_sub(1, Ordering::SeqCst);
    panic!("Max weak count exceeded");
}
```
The above code does not work, since two threads could execute 'fetch_add' simultaneously,
and the second thread might then see a count which equals MAX_WEAK_COUNT + 1.

One way to solve this is to do what the rust standard library does for Arc refcounts:
Reserve a range of counts to serve as a sort of 'overrun-area':

```rust
let count = item.fetch_add(1, Ordering::SeqCst);
if count >= MAX_WEAK_COUNT/2 { // Only use half the 'available' count range
    item.fetch_sub(1, Ordering::SeqCst);
    panic!("Max weak count exceeded");
}
```

The above works as long as at most MAX_WEAK_COUNT/2 threads execute the code simultaneously.
For std library 'Arc', the number of threads needed to trigger a bug on a 64-bit machine is 2^63, 
which is something that will clearly never happen in practice.

For ArcShiftLight, MAX_WEAK_COUNT is only 2^19, and half of that (2^18 = 262144) is a large, but realistically possible
number of threads. The 64-bit linux machine I'm writing this on has 

```bash
> cat /proc/sys/kernel/threads-max
506892
```

Because of this, ArcShiftLight does not use atomic 'add' to increase the weak count, but instead uses
`AtomicUsize::compare_exchange` instead. 

For the strong count, for ArcShift, the 'overrun-area' approach is used
instead. The overrun-area is about 180 billion strong counts.


## Unsafe rust is hard

The nodes holding payload values in Arcshift look like this:

```rust
struct ItemHolder<T: 'static> {
   next_and_state: AtomicPtr<ItemHolder<T>>,
   refcount: AtomicUsize,
   payload: T,
}
```

When there are no more strong links to an item, the field 'payload' is dropped.

Care is taken to not do any accesses to 'payload' after this point. However, 
it turns out, this is not enough - at least according to miri.

The problem is that code like this:

```rust 
    let ptr: *const ItemHolder<T> = ...;
    let count = unsafe{&*ptr}.refcount.load(Ordering::SeqCst);
```

... is not valid while 'payload' is being dropped. This is because dereferencing `*item` is
not allowed while one of its fields is being written to.

Instead, one must do:

```rust
    unsafe { &*addr_of!((*ptr).refcount) }.load(Ordering::SeqCst)
```
This is valid, since 'addr_of' does not dereference ptr. Critics of rust might point out
that this kind of code is more complex than equivalent C++-code. However, the beauty is that
this complexity is entirely contained inside ArcShift, and once ArcShift has been validated,
no one else ever has to think about it again.


## Miri is a fantastic tool

I've used miri extensively in the past, but I didn't appreciate that is not only a powerful
tool to ensure soundness of single threaded unsafe code, but that it is also very useful for
finding race conditions in multithreaded unsafe code. By using the `--many-seeds` option to
miri, miri will run the same test case multiple times, with different scheduling of the
program threads. By default (at time of writing), `--many-seeds` defaults to 64 different schedulings.
The set of seeds can be controlled by specifying a range, like `--many-seeds=0..1000`, to try
more variants.

One problem I encountered was in troubleshooting race conditions. Miri will provide a lot of useful
information, like where the leaked memory was allocated. However, this was often not much of a clue
in the arcshift test bench. The test bench has a lot of debug output, giving the memory address
of each allocated node. However, miri does not print the address of leaked memory, making it harder
to correlate the detected leak with the debug trace of the test run.

I finally solved this by reference counting the test payloads, so that most memory leaks could be
detected without using miri.

## Loom is a fantastic tool

Loom is a tool to detect threading errors in rust code. See https://crates.io/crates/loom .

By default, loom is exhaustive. That means it will check all possible interleavings of your 
different threads. This brings great piece of mind. However, as Arcshift accumulated features,
it became too complex to test using loom. Test cases involving 3 different threads are feasible,
but test cases with 4 threads simply take too long to run. 

Loom has a very convenient feature for replaying failed runs. An interesting thing with loom
is that it seems to 'emulate' actual multithreading by switching stacks, rather than actually
running multiple threads.

Another very powerful feature of loom (shared with miri), is that it can model non SeqCst orderings.
This confused me a lot at one point. Loom discovered bugs where threads read values which had obviously
been previously assigned other values. This is, however, allowed by the rust(=c++) memory model, whenever
SeqCst-ordering is not used. Internally, loom seems to maintain a list of possible values which
can be read from an atomic variable at each point.

Loom does not support SeqCst operation on atomic variables. However, inserting full memory
fences (loom::sync::atomic::fence) can be used to achieve the same result.

## Shuttle is a fantastic tool

Shuttle is a tool to detect threading errors in rust code. See https://crates.io/crates/shuttle .

Shuttle is a little bit like loom, but uses randomized scheduling instead of exhaustively trying
all possible interleavings. In contrast with loom, shuttle *only* supports SeqCst ordering. 

This means that shuttle is less powerful than loom. However, it can handle larger models, since
it is faster and less ambitious. Just like loom it supports replaying found failing execution traces.


## Cargo mutants is really cool

Cargo mutants is a tool which can be used to ensure that a test bench has enough coverage.

It works by modifying the code under test, and ensuring that the test bench fails.

Having code pass 'cargo mutants' is a lot of work. First of all, test coverage must be near 100%
But this is not enough - the test bench must also fail under the modifications done by cargo mutants.

Cargo mutants is quite hard to work with, but it does definitely bring something unique to the table.
For smaller code bases with very high ideals for correctness (like arcshift), it provides a lot of value.
However, I'm not convinced having a policy of 'always pass cargo mutants check' is suitable for
all code bases.

### Cargo mutants, challenge #1
One challenge I had with cargo mutants was with the following code:

```rust 
match get_next_and_state(candidate).compare_exchange(...)
{
   Ok(_) => {/* handle success */ } 
   Err(other) => {
       if !is_superseded_by_tentative(get_state(other)) {
           // Race condition, but still can advance
           candidate = other;
           continue;
       } else {
           // Race condition, need to retry
           continue;
       }
   }
}
```

In the Err-branch, we need to run another iteration of the algorithm. But because of the logic
of things, we can make more or less progress. Executing the first leg of the if-statement
gives us slightly better performance. I.e, the code is an optimization. However, 'cargo mutants' 
notices that the test bench  passes even if the if-condition is changed to 'false'.

One solution to this problem would be to introduce a performance test case, to make it so that
the test case fails if the optimization here is removed. However, measuring the performance
is going to be difficult.

One could argue that if there is no measured performance benefit, the optimization should be removed.
However, that argument doesn't sit entirely right with me. In the end, I added an exception for
this line to the cargo mutants-config.

### Cargo mutants, challenge #2

Another challenge is how to handle overflow of ArcShift-instances. The problem is that cargo
mutants identifies that removing the code for handling ArcShift instance count overflow doesn't
fail the test suite. However, triggering such an overflow would require creating 2*45 instances 
of ArcShift. Unfortunately, this requires at least 280 TB of RAM.

In the end, I just added an exception for this logic.

## Thoughts on focusing too much on failing test cases

Having a very exhaustive test bench is of great value. However, a problem that I encountered
at least twice while working on Arcshift, is that I lost track of the big picture, and entered a cycle of:

1. Run tests
2. Test fails
3. Fix particular failure of test case by adding more code
4. Repeat from #1

After a few iterations of the above loop, the code became very complex, and I was no longer really
sure of what invariants were supposed to hold.

It doesn't matter how comprehensive your test-suite is if you can never get it to pass :-) .

One part of the design that I should have nailed down instead of chasing breaking test cases was
the semantics of weak/strong refcounts.

In arcshift, the rules are like this:

1: Each ArcShiftLight-instance holds a weak (value 1) refcount on its primary node.
2: Each ArcShift-instance holds a strong (value 2^19) refcount on its primary node.
3: Each node with a live (non-dropped) payload holds a strong refcount on its next node (if it has one).
4: Each node with a dropped payload must have a next node.
5: Each node with a dropped payload holds a weak refcount on its next node.
6: When attaining a strong reference to a node, its 'next' pointer must be checked after increasing the refcount.
   If 'next' has a value, it must be used instead (decreasing and increasing refcounts appropriately).
7: When deciding to drop a payload, the node must be marked as dropped (affecting the next-ptr), and then 
   the refcount must be checked. If there are strong counts, the drop must be undone. Rule 6&7 ensure
   that there can never be strong links to a node with dropped payload
8: Because a node with a non-dropped payload always has a strong link to the next node, that next
   node cannot be dropped. By induction, all nodes 'to the right of' a non-dropped node are also non-dropped.

Before nailing these rules down, I had a very hard time fixing bugs where ArcShift and ArcShiftLight
instances were being reloaded and dropped simultaneously.













