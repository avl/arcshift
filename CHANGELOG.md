## 0.4.7
Fix a soundness issue in `cell::ArcShiftCell`: dereferencing an `ArcShiftCellHandle` multiple
times and retaining an earlier reference past a later `deref` could be a use-after-free, since
`deref` could reload the cell and drop the value the earlier reference pointed into.

## 0.4.6
Fix a panic in some code paths using unsized payloads (for example, `ArcShift<str>`).
This is not a soundness issue.

## 0.4.5
Fix two critical soundness issues:
 * The ArcShiftWeak type had the wrong variance. It was covariant, which made it possible
   to use safe code to extend the lifetime of an arbitrary non-static lifetime to static.
   Most use-cases for ArcShift use it with a 'static object, so this was not an issue
   for most users.
 * `ArcShiftWeak::upgrade` could erroneously return a dropped object. 
   This happened if the last strong instance of an ArcShift chain was dropped
   concurrently with a weak reference being upgraded, while a total of at least
   two weak refs existed.   
Disclosure: These bugs were found using Anthropic's Fable 5 model, but the fixes
were carefully validated by a human. 
 
## 0.4.4
Fix error in test suite, formatting test wasn't run correctly in loom.

## 0.4.3

Fix Debug impl of ArcShift, so that it honors "{:#?}" format strings.

## 0.4.2

Support rust 1.75 (0.4.1 regressed to require newer rust).

## 0.4.1

Support 32-bit platforms. Prior to this version, ArcShift did not provide correct information
to the allocator when deallocating heap blocks on 32-bit platforms.

## 0.4.0

`ArcShift<T>` now implements `Default` if `T:Default`.
