# Pushing integers end-to-end to runtime is the forcing function that validates the foundation

*2026-07-11*

**What happened.** After the columns rewrite stood up its thin vertical slice — `(def (main) 42)` compiling
to a running component on the real substrate — the operator's next push was not to add breadth (records,
sums, functions, effects) but to take *one scalar type all the way through to runtime with full support*:
integer literals, then generic arithmetic folded at compile time (I1), then genuine runtime operands crossing
the component boundary (I2b), then the full width family with signed/unsigned selection (I3, R1), faithful
boundary value types (R2), and a truncating conversion (R3). The instinct proved decisive. In the operator's
words: *pushing integers all the way through to runtime drove thinking about things as a whole, and about what
we would need before layering anything else on that would need to be ripped out.*

The reason a single scalar type forces whole-system thinking is that a thin scalar slice that only *folds*
answers a program never actually exercises the boundary, the runtime trap contract, or the machine
representation — every constant folds away and nothing crosses the wire. Only when a value has to survive to
run time does the compiler confront the questions that a from-scratch build is otherwise tempted to defer,
each of which, deferred, becomes a retrofit that rips out work built on top of the wrong answer:

1. **Signedness is a unification *variable*, not a fixed field.** The tempting model is that an integer type
   carries a fixed `(signed, width)` pair and a bare literal defaults to signed. That model clashes the moment
   an annotation grounds a literal the other way: `(: 200 UInt8)` fails against a `signed:true` default. The
   answer that generalizes is to make signedness a three-state unification axis — fixed, deferred, or a
   variable — *exactly mirroring width* (`enum Sign { Fixed(bool), Deferred, Var(u32) }` at `ty.rs:55`, paired
   with `Width` in `IntTy` at `ty.rs:66`; `unify_sign` conflicts two fixed signs into CDZ0301 and grounds a
   variable or deferred at `unify.rs:133`). A literal is then polymorphic in *both* axes, and the language rule
   "annotations constrain, never contradict" *falls out of ordinary unification* with no literal special-case.
   The operator asked directly — "is sign-as-a-variable right, or is your range special-case better?" — and the
   special-case (tried first) did not generalize; the unification variable did. This makes the arithmetic
   operators sign-generic for free: `scheme_of` pairs a fresh sign variable with each width parameter
   (`eval.rs:44`), so `+ : ∀(w,s). Int^s_w → Int^s_w → Int^s_w` serves signed and unsigned from one prelude
   entry and an `Int64`/`UInt64` mix is a sign conflict, not a coercion.

2. **A runtime arithmetic operation must *trap* on overflow, not silently wrap.** The const-fold side of this
   was already right (a compile-provable overflow poisons the build, CDZ0304). But I2b's first runtime emit was
   a bare `i64.add` — which is wasm's *silent wrap* — a miscompile invisible until an exported parameterized
   function actually ran with overflowing operands (`add(MAX,1)` returned `MIN`). The fix is a width-generic
   checked-emit recipe: a value of width `N` is promoted to the smallest machine slot that holds it (i32 for
   `N ≤ 32`, else i64 — `Machine` at `select.rs:264`), the op emits its machine instruction plus a signed
   overflow guard, plus a range-check back to `[min_N, max_N]` when the width is narrower than its slot
   (`select.rs:499`, `593`). Together they trap iff the true result leaves the N-bit type — `Int8` `100+100`,
   `UInt48` `*` past `2⁴⁸`, and `Int64` `+` past `2⁶³` all by one recipe, no hard-coded width.

3. **A type with no wire form declines at the boundary.** A narrow non-standard width (`UInt7`, `UInt24`,
   `UInt48`) is a perfectly good *internal* type but has no component primitive to cross as. `comp_valtype_of`
   returns a distinct primitive for each aliased width and `None` otherwise (`lir.rs:213`); at the boundary a
   `None` is a *decline* asking for an explicit conversion (`serialize.rs:195`), not a silent widening. The
   safety property is the operator's framing: the host never sees "7 bits smuggled in a byte that might hold
   200" — a boundary type is one wasmtime can enforce at the edge.

4. **A truncating conversion is *one* primitive whose target is read off the solved type, and it never traps.**
   The naive model is a conversion operation per (source, target) pair — an O(widths²) explosion. Instead there
   is a single `Prim::Wrap` (`resolved.rs:92`) typed `∀(w,s). Int^s_w → T`, fully generic in its source; the
   *target* width is read off the application's already-solved type at lowering (`lower.rs:363`), so O(widths)
   prelude entries share one primitive and one lowering. The operator's rule fixed a policy at the same time:
   `.wrap` *never traps* — "avoid sharp edges in the language; accidental truncation is too sharp." Traps are
   reserved for arithmetic overflow (almost always a bug); a genuinely fallible narrowing returns `Option` via a
   separate checked `.of`. This retired the ad-hoc `to-byte` op: `UInt8.wrap` *is* it, with the width read from
   the type rather than baked into a magic name.

The through-line the slice also revealed: **a genuine runtime integer operand only exists behind an exported
parameterized function.** Every call to a user function *folds* at compile time (the monomorphization tier —
`((fn (x) …) 5)` β-reduces to the substituted body), and a nullary-with-constants entrypoint folds to a
constant. The *only* program shape that forces an integer to survive to run time is an export that takes a
parameter. That is why the whole integer arc reached runtime **without runtime user functions or recursion
existing at all** — those still decline today (`eval.rs:223`, no `Core::Call` in the core). The vertical slice
proved the boundary, the trap contract, and the machine representation *independently of the call machinery*,
which is exactly the point: the foundation was validated before the features that would have sat on top of a
wrong answer were built.

**Why.** The alternative to a deep vertical slice is a broad shallow one — get records, sums, functions, and
effects each half-built at the fold level — and it is a trap, because none of them is *validated* (nothing
runs the boundary or the trap contract) and each is built on representation and boundary decisions that turn
out wrong once a value has to run. Overflow-wraps-silently, sign-is-a-fixed-field, every-width-crosses-the-wire,
and a-conversion-per-type-pair are all locally reasonable and all get ripped out the moment a real value
crosses to run time. Driving *one* type to genuine runtime completeness surfaces every one of them while the
core is still at its thinnest and the cost of getting the answer right is a single prelude entry or one shared
lowering — not a cross-cutting refactor of a dozen half-built features. The corpus makes the payoff concrete:
each increment landed as prelude entries and shared lowerings on the *existing* one-`(meta apply)` path with no
new IR rung and no resolver special-case, and the gate climbed monotonically (48 → 115 pass, 0 fail) with every
increment byte-neutral or byte-additive. That "add a feature = add a map entry" experience *is* the
solid-foundation dividend — and it is only collectable because the substrate (records-everywhere, solve-once,
the columns model, the one evaluator) was built first and then stress-tested by a value that had to run.

This reframes the build order. The reproduction plan is right that integer *breadth* (many widths, numeric
completeness) is cheap late-stage work. But *depth* — one scalar type end-to-end to runtime — is not late-stage
breadth; it is an early **forcing function** that validates the foundation, and it belongs right after the core
and the one evaluator exist, before patterns, the completed backend, functions, and effects are layered on. A
build that adds breadth before it has driven a single value to runtime completeness is building on unvalidated
boundary and representation decisions.

**The requirements it drove.** Folded into the architecture (naming no engine or width per
[constitution §XIII](../../constitution.md); the concrete grounding lives here):

- [build-order.md](../architecture/build-order.md) — the *deep vertical slice is a forcing function* principle
  added to §The Two Forces, and a new early stage (**One Scalar Type End-To-End To Runtime**) inserted after
  the core-and-evaluator stage and before patterns/backend/functions/effects, distinguished from the late-stage
  *width breadth* it de-risks; new watch-outs for the four traps above and for the runtime-operand-needs-an-export
  reality.
- [reference-compiler.md](../architecture/reference-compiler.md) — §Instruction Selection gains *a runtime
  arithmetic operation traps on overflow through a bounded guard* (the runtime sibling of the compile-provable-trap
  rule) and *a truncating conversion is one operation whose target is its solved type*; §The Component Boundary
  Is Explicit Data gains *a type without a boundary representation declines at the boundary*.
- [prelude-and-resolution.md](../architecture/prelude-and-resolution.md) — §A Numeric Width Is A Type Record
  gains that a type's signedness, like its width, is a value determined by unification (a literal leaves both
  open and use or annotation grounds them), so "annotations constrain, never contradict" is a consequence of
  unification rather than a literal special-case.

The *semantic* sentences this arc also implies — that the truncating conversion truncates the low bits and
never traps, that only the aliased widths have a boundary form, that a fallible narrowing returns the absent
case — are [numeric-model.md](../capabilities/numeric-model.md) folds, left for a separate, gate-timed
capability pass rather than pulled into the (unregistered) architecture documents here.
