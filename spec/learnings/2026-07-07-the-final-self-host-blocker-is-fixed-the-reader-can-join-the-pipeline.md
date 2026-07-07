# The final self-host blocker is fixed — the reader can now join the pipeline, and the scale-limit case became pinnable

*2026-07-07*

**What happened.** Tier 2f — the "runtime compound element of a kind the runtime cannot box yet"
decline that blocked feeding a runtime-built `Node` to the real `resolve : Node → Core`
([[2026-07-07-the-final-self-host-blocker-is-a-scale-limit-not-a-shape-gap.md]], SPEC-BACKLOG item 16) —
is **fixed in the seed**. Verified directly: the full 18-variant `resolve` shape (a `Node` with a
String head, dispatched through `head-prim` to a 15-variant `Prim`, returning the 18-variant `Core`),
applied to a runtime-built `Node` and scalar-consumed, now runs (→ 1) where it previously declined. The
runtime heap-boxer now admits the element-kind combination the full `Core` union produces on the
`resolve` path. This was the single remaining hard blocker on `bytes → bytes` self-hosting: the reader
(`read-node : Bytes → Node`, already verified as a Node builder) can now be joined to the existing
pipeline, so `read → resolve → fold → lower → serialize → frame` is unblocked at the seed level. Every
self-hosting blocker the spike found — Tier 00 (exponential inlining), Tier 0 (runtime strings), 2b
(nested payload binder), 2c (`Bytes.at` Option), 2d (bare nullary + recursive-Bool kind race), 2e
(runtime `tuple.N`), 3a (`match` shape inference), and now 2f — is cleared.

Because 2f is fixed, the corpus case that could **not** be pinned last cycle now can. Last cycle 2f was
a *scale limit* with no minimal witness — every tractable resolver passed, only the full 18-variant one
failed — so the honest artifact was a bisected backlog entry, not a corpus case
([[2026-07-07-the-final-self-host-blocker-is-a-scale-limit-not-a-shape-gap.md]]). Now that the seed
handles the full union, a *representative* recursive `Node → Core` resolver is a durable green
regression guard: it exercises the reader→pipeline join shape (a runtime sum transformed into a
different runtime sum, then consumed) and passes, so it pins the capability without needing to
reproduce the exact former threshold.

**Why.** The lesson is the flip side of last cycle's rule. A scale limit resists a minimal corpus case
*while it is broken* (every reduction passes, so nothing small witnesses the failure); but *once fixed*,
a representative case at natural size becomes a perfectly good regression guard — it no longer has to
straddle a fragile threshold, it just has to exercise the shape and pass. So the full lifecycle for a
scale limit is: **while broken, bisect + backlog (no corpus case, because none is minimal); once fixed,
pin a representative case (not the giant threshold case, just one that exercises the shape).** The
regression guard for the *whole* capability remains the real artifact — `compiler.cdz` connecting
`read → resolve → … → frame` and compiling under the two-compilers gate — but a representative corpus
case is the portable, feature-level guard that belongs in the executable semantics. This also confirms
the two-compilers architecture paying off exactly as intended: authoring the compiler drove out a
sequence of runtime-heap and inference gaps (nested binders, Option-across-boundary, recursive-Bool
kinds, tuple projection, the Core-union box limit) that a floor-outward corpus never would have, and
each fix landed as either a pinned corpus case or a bisected backlog entry — the language grew to meet
its most demanding program.

**The requirement it drove.** A conformance case in `05-compound-types.sexp` — *"a recursive resolver
transforms one runtime sum tree into another, then consumes it"* — pins the reader→pipeline join shape:
`resolve : Node → Core` maps a runtime-built `Node` (String-headed, name-dispatched) to a typed `Core`,
and `eval : Core → Int64` folds it, `resolve (NPrim "+" (NInt 20) (NPrim "*" (NInt 2) (NInt 11)))` →
`eval` → 42. It is deliberately a **cross-sum-type** transform (Node → Core → scalar), distinct from the
existing `Expr` self-evaluator (which stays within one type): the intermediate `Core` is a genuine
runtime value the producer materializes and the consumer walks — the exact shape 2f was blocking. It
**PASSES**, turning the former final blocker into a permanent gate obligation and closing item 16. With
every self-hosting seed blocker now cleared, the remaining work is *wiring* (committing the
`read-node → resolve` join in `compiler.cdz`, which the spike had kept uncommitted until 2f landed) plus
the two non-blocking items — 12 (symbol-table `from-bytes`, which the reader routes around for
structure) and 13 (list patterns, ergonomic). The handoff docs still lag (SEED-GAPS Tier 2f and
`compiler.cdz`'s header both carry pre-fix text — the third and fourth stale claims this session), which
is why the probe, not the doc, confirmed the fix.
