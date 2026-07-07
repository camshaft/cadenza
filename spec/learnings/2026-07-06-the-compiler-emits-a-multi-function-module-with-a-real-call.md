# The compiler emits a multi-function module with a real call — and routes around the front-rung blocker to prove the backend

*2026-07-06*

**What happened.** The compiler-in-Cadenza spike reached a real milestone: `compiler.cdz` now compiles
to a valid WebAssembly component that is **more than one function** and threads a **real `call` with a
parameter**. The target program is `main = dbl(21)`, `dbl x = (+ x x)` — func 0 (nullary `main`) calls
func 1 (unary `dbl`) with the literal 21, and `run` yields 42 through an actual `call 1`, not a
compile-time fold. Reaching it required three new pieces in the resolved-IR ladder, each following the
patterns the earlier stages established: a multi-function assembler (`compile-program` over an `FList`
of `Func`, emitting the N-entry type / function / code sections and one functype per function), and two
new `Core` constructors — `KLocal n` (a parameter read, lowering to `local.get n`) and `KCall (fi, arg)`
(lowering to `arg-code ++ call fi`). The runtime conditional also matured: `KIf` now lowers to a
**structured `if/else/end`** whose blocktype is the branches' shared result `Kind`, so a conditional
selected by a *runtime* value (not a fold) emits a genuine two-way branch.

But the headline is *how* the milestone was reached: **the agent routed around the front-rung blocker
rather than waiting on it.** The front rung `resolve` still declines on the nested tuple binder
(`(NPrim (tuple p (tuple a b)))`, Tier 2b — [[2026-07-06-the-front-rung-of-a-resolved-ir-compiler-needs-nested-payload-binders]]),
so `main` no longer feeds `resolve` a runtime `Node` tree. Instead it hand-builds the **folded `Core` /
`Func` list directly** — exactly what `resolve` *would* produce — and drives the assembler from there.
The compiler is thus validated **from the resolved IR inward** (assemble → frame a multi-function
module, with calls, params, and runtime conditionals all emitting correct bytes) while the front rung
stays stubbed behind the seed gap.

**Why.** This is a deliberate, healthy sequencing move, and it is worth recording as a *method*, not
just a status update. A resolved-IR compiler has a clean seam between the front end (surface → resolved
`Core`) and everything downstream (analyze → lower → serialize → frame). When the front end is blocked
on a seed gap, feeding hand-built `Core` at that seam exercises the entire downstream — which is most of
the compiler and all of the byte-emitting risk — without waiting for the blocker to clear. The seam that
[[2026-07-06-lower-through-a-resolved-ir-so-emission-is-a-serializer]] introduced for *architectural*
reasons (emission serializes a resolved form) turns out to also be the right *testing* seam: because
`Core` is an ordinary user sum value, `main` can construct it by hand, so the backend is provable before
the front rung compiles. The cost is honesty about what is proven: the emitted bytes are correct *given*
a resolved tree, but the surface→`Core` rung (name/opcode resolution, the nested-binder decode) is not
yet exercised end-to-end — it is stubbed, and the stub must be replaced, not forgotten. The milestone is
"the backend assembles a real multi-function program," not "the compiler is self-hosting"; conflating
the two would be the modeled-derivation trap ([[2026-07-02-a-modeled-subsystem-passes-a-shape-check]]).
The blocker is unchanged and still gates the front end: **Tier 2b / pattern-binder nesting remains
SPEC-BACKLOG item 1**, now demonstrably the *only* thing between the working backend and a compiler
driven from a real surface tree.

**The requirement it drove.** Two conformance cases in `02-binding-and-control.sexp` pin the newest
emit behavior — the runtime conditional: *"a conditional selects a branch by a runtime value that is
not known at compile time"* (`(def (f x) (if (< x 10) x (* x 2)))`, `f(21) → 42` via the else-branch)
and its then-branch companion (`f(3) → 3`). Every prior conditional case had a compile-time-known
condition (a literal, a nested `if`, or a foldable comparison, including the shielding cases); these are
the first to force a genuine runtime two-way branch — the `KIf → if/else/end` structured lowering the
compiler now emits — so the pair pins that selection happens at run time by the condition's value, not
by a fold. Both PASS. The multi-function-with-a-real-`call` shape and the `local.get` parameter read are
already covered by `09-functions.sexp` (application, recursion, currying) at the source level; what this
milestone adds is that the Cadenza-authored compiler *emits* that shape correctly from hand-built `Core`,
which the two-compilers gate will pin once the front rung is unblocked and the whole path runs from a
surface program. Until then, the honest status is recorded here and in the spike handoff: backend proven
from the resolved IR inward; front rung blocked on Tier 2b (backlog item 1).
