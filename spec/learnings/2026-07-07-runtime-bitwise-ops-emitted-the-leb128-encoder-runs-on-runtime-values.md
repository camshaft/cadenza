# Runtime bitwise `&`/`|` are emitted — the compiler's own LEB128 encoder now runs on runtime values

*2026-07-07*

**What happened.** Subset-growth toward self-inclusion (backlog item 20): the emit-side `Core` gained
`KBitAnd`/`KBitOr`, so **runtime bitwise AND/OR** — the value arriving through a function parameter, not
a constant — now compile and run. Verified: `(def (lo7 n) (& n 127)) (lo7 200)` → 72; `(| b 128)` on a
runtime `b` → 133; and the composed LEB128 byte `(Int.to-byte (| (& n 127) 128))` on a runtime `n=300`
→ 172. Previously these ops were const-fold-only on the emit path (the byte-extraction worked on
literals but not on a value computed at run time).

**Why.** This matters specifically because the compiler's own LEB128 encoder is written this way. Every
section length, vector count, and operand the compiler emits is a value it computes at *run time* and
then LEB128-encodes with exactly `(| (& n 127) 128)` / `(& (>> n 7) 127)` — so a self-hosted compiler
cannot encode its own output until runtime `&`/`|` are emitted, not just const-folded. It is another
entry on the emit-coverage checklist ([[2026-07-07-match-on-user-sums-is-the-last-major-emit-frontier]])
— the operator surface the compiler's source uses, filled in one op-family at a time. And it is the
recurring **const-masks-the-runtime-gap** trap once more: the corpus had `&`/`|` cases, but on
*constant* operands (`(& 255 127)`, `(| 42 128)`), which const-fold and so never exercised the emitted
`i64.and`/`i64.or` — exactly the pattern where a passing literal case hides a missing runtime emitter
([[2026-07-07-the-workaround-was-the-bug-correcting-the-scale-limit-diagnosis]] /
[[runtime-list-at-fallible-index]]). The lesson holds yet again: **a const case for an operator is not
evidence the operator emits; the runtime-through-a-parameter case is.** The compiler's LEB128 encoder is
the sharpest witness — its whole job is to run the bit ops on runtime values.

**The requirement it drove.** A conformance case in `06-numeric-model.sexp` — *"the LEB128 byte
composition runs on a runtime operand"* — pins the emitted path: `(leb-byte n) = (Int.to-byte (| (& n
127) 128))` on a runtime `n=300` → 172, the same non-final byte the constant composition produces but
reached through the emitted `i64.and`/`i64.or`. It is deliberately the runtime-through-a-parameter
companion of the existing constant LEB128-byte cases (which fold and cannot witness the emitted bitwise
path), and it PASSES. With runtime `&`/`|` covered, the compiler's LEB128 encoder — the spine of its
byte output — runs on the runtime values it is actually fed, not only on literals. This is subset-growth
progress on backlog item 20's operator coverage; the standing frontier is unchanged — the last major
emit item is still `match` on user sums (the emit-side `Core` still has no `KMatch`), plus scale (TCO).
No new backlog item; the runtime-vs-const emitter distinction is the durable methodological note,
already recorded as a standing rule.
