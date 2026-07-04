# Exhaustion is observed as a trap across the compiled seam

*2026-07-03*

**What happened.** A differential gate was built that runs every realized executable-semantics case
through both the reference interpreter (the oracle) and the Cadenza-authored compiler → WebAssembly
component → run, and compares observable behavior (agree / todo / skip / disagree). While planning to
grow the compiler to emit functions and recursion, a seam mismatch surfaced: the interpreter halts
unbounded recursion in the distinct terminal condition `exhausted` (core-semantics.md §"A Program
Terminates In Exactly One Terminal Condition"), but a running WebAssembly component has no separate
"exhausted" outcome — a bounded halt reaches the host as a trap. Left unresolved, the two recursion
corpus cases (`03-equality-and-observation.sexp` and `09-functions.sexp`) would flip from `todo` to a
false `disagree` the moment the compiler emitted recursion, even though the compiled behavior is
correct — it halts deterministically, just under the name "trap".

**Why.** The observable-behavior projection distinguishes three terminal conditions, which is right for
the interpreter (the oracle can name exhaustion precisely). But the *component boundary* is coarser: it
signals any bounded halt as a trap and carries no third outcome. Oracle agreement is a statement about
observable behavior across that boundary, so the projection compared across the seam must be the one the
boundary can actually express. Nothing in the spec had said how the interpreter's `exhausted` maps onto
a derived component's outcomes, so "agreement" was underdetermined exactly where the frozen
`determinism-and-fuel.md` contract already fixed the *emission* side (§"Exhaustion Halts Deterministically"
requires a deterministic halt point; it does not name the outcome the host observes). The operator
resolved it: across the compiled seam, exhaustion is observed as a trap.

**The requirement it drove.** Added `spec/capabilities/self-hosting-and-bootstrap.md` §"Exhaustion Is
Observed As A Trap In A Derived Component" (two requirements: a derived component that exhausts the
resource measure MUST halt as a trap and be judged as agreeing with the oracle's exhaustion terminal
condition; and a derived component MUST NOT be required to distinguish exhaustion from a trap in its
observable behavior). This is additive — it constrains how the existing oracle-agreement obligation
(§"A Derived Component Agrees With The Oracle", constitution §XIV) is checked, and leaves the frozen
`determinism-and-fuel.md` contract untouched (that contract still governs emission: every function call
and loop iteration consumes the measure and exhaustion halts at a deterministic point). The differential
gate's comparison treats oracle-`exhausted` vs component-`trap` as agreement accordingly.
