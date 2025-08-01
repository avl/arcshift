## 0.4.1

Support 32-bit platforms. Prior to this version, ArcShift did not provide correct information
to the allocator when deallocating heap blocks on 32-bit platforms.

## 0.4.0

`ArcShift<T>` now implements `Default` if `T:Default`.
