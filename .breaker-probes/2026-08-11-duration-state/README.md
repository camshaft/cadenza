# Duration newtype state (2026-08-11)

Angle: user newtype (Duration UInt64) as handler state — unwrap/arith/rewrap
per dispatch at nanosecond scale, and the u64-MAX overflow face.

GREEN x3:
- du1: seconds accumulate across dispatches (unwrap, add ns, rewrap) — 703
- du2: the state sits at MAX-1; a runtime +1 crosses on dispatch 2 -> traps
  integer overflow uniformly; +0 threads unchanged — 0/trap

Notes: UInt64 has NO to-int — Int64.of converts; a compile-PROVABLE overflow
is CDZ0304 at compile time (the +1-literal draft), so the trap face needs a
runtime-dependent operand.

Pin candidates: 246 pool.
