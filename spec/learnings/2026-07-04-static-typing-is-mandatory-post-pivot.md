# Static typing is mandatory once the seed is a compiler; the dynamic carve-out is retired

*2026-07-04*

**What happened.** Constitution Amendment 0.4.0 retires the Principle VII bootstrap carve-out
(Amendment 0.2.0). The seed compiler must now enforce static typing — reject ill-typed programs at
compile time — rather than defer it.

**Why.** The 0.2.0 carve-out was explicitly conditioned on *realizing evaluation dynamically*: "the
operator-synthesized seed generation MAY defer static typing **and realize evaluation dynamically**."
A tree-walking interpreter has a defined dynamic outcome for an ill-typed program (it traps at
runtime), so deferring the static check was sound *for that shape of seed*. The two-compiler pivot
(Amendment 0.3.0) removed the dynamic evaluator entirely: cdz-rustc **compiles** a program to a
WebAssembly component and runs *that*. There is no dynamic evaluation left to lean on. Emitting a
component for an ill-typed program would either miscompile it or push a type error into the runtime —
exactly what Principle VII forbids ("MUST reject … rather than emit a component carrying a deferred
type error"). So the premise of the carve-out is void, and VII applies in full.

**The inversion this forces.** The executable-semantics corpus records, for programs that a dynamic
interpreter runs but a typed compiler rejects, BOTH clauses: the interpreter's dynamic primary clause
(a trap or a value) AND an inline `(compiler (error CDZ####))`. Under the dynamic seed, cdz-rustc's
predecessor matched the *primary* clause. Under a compiling seed, cdz-rustc must match the
**`(compiler …)`** clause where one exists. Concretely:
- `(= (Point (x 0) (y 0)) (Vector (x 0) (y 0)))` → **CDZ0202** (comparing distinct nominal types is a
  type error), NOT the dynamic `true`. Comparing across a nominal boundary is the whole reason to name
  a type — the values are not interchangeable, so the comparison is not valid.
- `(+ 2 2.0)` → **CDZ0301** (numeric types do not silently promote); `(+ 1 "two")` → **CDZ0201**.
- a non-exhaustive match → **CDZ0210**; an annotation contradicting the value → **CDZ0203**; reaching
  an undeclared host operation → **CDZ0401**.
These stop being dynamic traps and become compile-time rejections cdz-rustc emits.

**Reject-don't-miscompile still bounds the growth.** VII now permits realizing the type rules
*incrementally* — a generation's compiler may not yet check every rule — but it MUST reject (decline)
rather than miscompile a program using an unchecked rule. This is the same discipline that already
keeps `todo` honest against `disagree`: an unhandled construct declines; it never emits divergent
bytes. So the type system grows rule-by-rule without ever shipping a deferred type error.

**What this means for cdz-rustc.** It gains a rejection channel: a compile-time `Reject(code)` outcome
distinct from `Decline` (not-yet-compiled) and from emitting a trapping component. The behavior gate,
which currently checks the interpreter's primary clause, must prefer a case's `(compiler …)` clause
when present and confirm cdz-rustc rejects with that code. Cases whose only clause is a dynamic trap
that a typed compiler would ALSO reject (numeric mismatch, applying a non-function to a known
non-function, non-exhaustive match) resolve to rejections; genuinely runtime traps that survive type
checking (integer overflow, division by zero, index out of bounds, resource exhaustion) remain
runtime traps in the emitted component.

**The requirements it drove.** [constitution.md](../../constitution.md) Core Principle VII amended
(Amendment 0.4.0): carve-out retired; seed compiler enforces typing; ill-typed → machine-readable
rejection code; incremental realization under reject-don't-miscompile. Supersedes
[2026-07-02-seed-is-a-dynamic-interpreter.md](./2026-07-02-seed-is-a-dynamic-interpreter.md) (that
learning's dynamic-seed premise no longer holds post-pivot). Composes with
[2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md](./2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md)
and [2026-07-04-nominal-record-is-tagged-structural-record.md](./2026-07-04-nominal-record-is-tagged-structural-record.md).
