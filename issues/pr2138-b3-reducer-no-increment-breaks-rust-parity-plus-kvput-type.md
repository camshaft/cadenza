# PR #2138 review — reducer_b3.cdz (v-harness-bootstrap) — OPEN — behavioral-parity (MED) + effect-sig (MED)

https://github.com/camshaft/cadenza/pull/2138 (the first REAL Cadenza agent-harness reducer programs B1-B3
+ Project.cdz). Copilot 2 inline on the flagship B3 reducer — both VERIFIED, both parity/correctness.

## B3's KV counter DOESN'T INCREMENT + wrong init byte → breaks behavioral parity with the reference `reducer-guest` (Copilot, reducer_b3.cdz:57) — correctness/parity [VERIFIED, MED]
> The KV counter logic doesn't match the referenced Rust fixture: it stores the previous bytes unchanged
> (no increment) and initializes the counter to ASCII "1" rather than a single byte with value 1. This
> breaks behavioral parity with `reducer-guest` (which reads the first byte and writes
> `prev.wrapping_add(1)`).

VERIFIED in the diff (reducer_b3.cdz:197-199): `Kv.put(String.to-bytes("count"), match Kv.get(String
.to-bytes("count")) with | Some(prev) => prev | None() => String.to-bytes("1"))`. Two parity breaks: (1)
the `Some(prev) => prev` arm stores the old bytes UNCHANGED — no increment at all (the doc comment right
above says "bump the KV counter (read old, write new)", but nothing bumps); (2) `None() =>
String.to-bytes("1")` inits to ASCII '1' (byte 0x31), not a single byte of VALUE 1. The reference
`reducer-guest` reads the first byte + writes `prev.wrapping_add(1)`. So this Cadenza reducer neither
increments nor matches the Rust byte semantics — defeating the behavioral-parity the whole B1-B3 bootstrap
exists to demonstrate (a Cadenza reducer that folds identically to the interim Rust one). MED. Fix: read
the first byte of `prev`, `wrapping_add(1)`, write the single-byte result; init to a byte of value 1 (not
"1"). Needs whatever byte-at/construct ops the Cadenza reducer surface exposes.

## `Kv.put`'s type uses a backticked `` `->`(Bytes, Bytes, Unit) `` form vs the repo's effect-sig convention (multi-arg ops take one `Tuple(...)`) → likely typecheck mismatch (Copilot, reducer_b3.cdz:31) — effect-sig [VERIFIED-plausible, MED]
> `Kv.put`'s type uses a backticked `->` form that doesn't match the effect-signature style used elsewhere
> in the repo (e.g. multi-arg effect ops take a single `Tuple(...)` argument). This is likely a syntax/type
> mismatch and will make the fixture fail to typecheck once `Kv.put` is called with a tuple.

VERIFIED the form (reducer_b3.cdz:31 `put : \`->\`(Bytes, Bytes, Unit)`) differs from sibling effect ops.
Whether it's a genuine typecheck break depends on the effect-op declaration convention (does a 2-arg effect
op take `(Bytes, Bytes)` positionally or one `Tuple(Bytes,Bytes)`?) — v-harness-bootstrap/v-inference know
the canonical shape. MED if it mis-typechecks (it's the first real reducer, so the fixture must compile).
Confirm the `Kv.put` sig matches how the kernel's kv effect is declared + how the call site passes args.
v-harness-bootstrap owns the reducer fixtures. (Both findings gate whether B1-B3 actually demonstrate a
working Cadenza agent — worth getting right on the flagship.)
