# An out-of-arity tuple index traps at runtime instead of being rejected

*2026-07-07*

**What happened.** Adversarial probing of the positional tuple accessor found a
compile-time-knowable ill-typing deferred to a runtime trap. `(tuple.3 (tuple 10 20 30))`
projects position 3 of a three-element tuple (valid positions 0..2). The compiler emits a
component that **traps at run time** rather than rejecting the program at compile time. The
tuple's arity is statically known here (a literal `(tuple 10 20 30)`), and it holds for
let-bound and conditionally-selected tuples too (`(let ((t (tuple 1 2))) (tuple.5 t))` traps;
`(tuple.5 (if … (tuple 1 2) (tuple 3 4)))` traps).

**Why it is a break.** type-system.md #A Tuple Is Split At A Position Into A Prefix And A
Suffix: "A split position that is not within the operand tuple's static arity range MUST be
rejected at compile time as a type error, consistent with a positional tuple access whose index
is out of the tuple's static arity being rejected." A tuple's arity is part of its type (a
fixed-size positional value), so an out-of-range index is a *static* type error, not a runtime
condition. The corpus already pins the sibling rule — `(tuple.0 5)` (a non-tuple operand) is
rejected CDZ0201 "rather than emit a component that traps" — and this is the arity companion of
exactly that principle. Emitting a trapping component for a statically ill-typed program is the
same "ill-typed program not rejected" class as the annotation and constructor-arity breaks.

**Contrast with member access, which correctly traps.** `(. r missing-field)` *traps* (a
recorded corpus behavior), because a record's field set can be runtime-dependent — the compiler
cannot always know statically whether a field is present. A tuple's arity, by contrast, is
always static. So the two accessors diverge on purpose: missing record field → trap; out-of-
arity tuple index → compile-time reject. The seed applied the record rule (defer to a runtime
trap) to the tuple accessor, where the stronger static rule holds.

**Root cause — the type-rejection pass checks the operand kind but not the index bound.** In the
seed (`codegen.rs`, the `head.starts_with("tuple.")` arm of `check_type_rejections`), the check
is `if static_type(operand) != Tuple { reject CDZ0201 }` — it confirms the operand is a tuple but
never compares the index N against the tuple's arity. So `(tuple.3 (tuple 10 20 30))` passes the
Tuple check and reaches codegen, which emits an `arr-get` that traps out of bounds. The fix is to
recover the operand tuple's static arity (available from its `Shape`) and reject when
`N >= arity`, alongside the existing non-tuple check.

**The lesson.** A "wrong-kind" guard and a "wrong-index" guard are two different checks, and
having the first is easy to mistake for having both. The accessor's type rule has two clauses —
the operand must be a tuple AND the index must be within its arity — and the seed implemented
only the first, letting the second fall through to the runtime. The tell is that the failure
*observable* (a trap) matched the record-accessor's defined behavior, which masked that the tuple
accessor has a stricter static contract. When two accessors share a lowering but differ in what
is statically knowable, the stricter one needs its own static check, not the shared runtime trap.

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"a positional tuple access out of
the tuple's static arity is a type error" — `(tuple.3 (tuple 10 20 30))` MUST reject CDZ0201, as
the arity companion to the existing non-tuple-operand cases. Native seed; the behavior gate
catches it (expected reject CDZ0201, observed a running component).
