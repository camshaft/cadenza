# Tuple projection of a function parameter emits an invalid component

*2026-07-08*

**What happened.** Adversarial probing of positional tuple access found the worst outcome class:
a compiler that emits an **invalid wasm component** (fails validation), not a trap or a wrong
value. `(def (fst t) (tuple.0 t))` applied to `(tuple 7 8)` produces a component that fails wasm
validation ("failed to compile: wasm[0]::function[44]"). The program is well-typed — its value is
7, and both the inline `(let ((t (tuple 7 8))) (tuple.0 t))` and the beta-reducing `((fn (t)
(tuple.0 t)) (tuple 7 8))` compute 7 correctly. Only `tuple.N` applied to a value that arrives as
a **named-function parameter** (a runtime tuple whose shape is not the inline literal at the
projection site) emits the invalid component. `tuple.1` does the same.

**Why it is a break.** self-hosting-and-bootstrap.md #An Unsupported Construct Is Declined, Not
Miscompiled: "A generation whose compiler does not yet compile a construct a program uses MUST
decline to derive a component … rather than emit a component whose observable behavior diverges."
An invalid component is neither a decline nor a valid component — it is the worst outcome, and the
constitution's never-crash / valid-or-rejected discipline forbids it. The compiler must either
compute the projection (the program is well-typed, value 7) or decline; it must never emit bytes
that fail validation.

**The record accessor already does the right thing — the tuple accessor is the outlier.** The
exact analogue for records, `(def (geta r) (. r a))` applied to a record parameter, correctly
DECLINES with "runtime member access on a value of unknown record shape." So the seed already
knows how to handle a compound projection whose operand shape is not statically recoverable at the
projection site — it declines. The positional tuple accessor `tuple.N` does not take that path: on
a parameter it proceeds into codegen with a shape it cannot honor and emits malformed bytes
(likely a `tuple.N` access lowered against a wrong/absent local layout, producing an ill-typed or
out-of-range wasm instruction that fails validation). The fix is to make `tuple.N` on an
unrecoverable-shape operand decline exactly as the record accessor does — or, better, recover the
parameter tuple's shape and compute (the program is well-typed).

**The lesson.** Two accessors that project a compound — `(. r field)` and `tuple.N` — share a
"what if the operand's shape isn't known here?" case, and they diverged on it: the record accessor
declines, the tuple accessor emits invalid bytes. When one projection form has a decline path for
an unrecoverable-shape operand, its sibling projection forms need the same path; the give-away was
that the record and tuple accessors, which are described as mirrors throughout the corpus ("`(. …)`
requires a record operand, exactly as `tuple.N` requires a tuple"), behaved oppositely on the
named-parameter operand — one safe, one emitting invalid wasm. A decline is the floor; emitting a
component that fails validation is below the floor.

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"projecting a tuple passed as a
function parameter yields the element, never an invalid component" — `(def (fst t) (tuple.0 t))`
applied to `(tuple 7 8)` MUST yield 7 (or decline), never emit an invalid component. Native seed;
the behavior gate catches it (expected output 7, observed "emitted invalid component: … failed to
compile"). A generation that cannot yet thread the parameter tuple's shape declines (scored todo);
emitting an invalid component FAILs the case.
