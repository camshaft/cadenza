# A fixpoint loop's compile blowup is the fresh-re-seed-plus-list-result conjunction — not the loop, and not either half alone

*2026-07-07*

**What happened.** The self-hosting return-kind machinery's next step is a monotone **fixpoint** — iterate a
table until it stops changing — and the handoff doc (`SEED-GAPS`) flagged that two fixpoint reproducers still
OOM the seed (multi-GB RSS, killed), where the just-landed single-pass accumulator fix was not enough. The doc
attributed the trigger to "a `list`-typed parameter REPLACED by a freshly-built `(list …)` each round." Probing
the seed directly (`emit`, `ulimit -v 4G`, 30–40s timeout) both **confirmed the blowup** and **narrowed the
trigger past the doc's description** with four controls:

| # | shape | result |
|---|-------|--------|
| (a) | `iterate (list) (- passes 1)` — fresh `(list)` re-seed each round, result `List.len`'d | **OOM (killed)** |
| (b) | `match`-driven `recompute` re-seeded with `(list)` inside a fixpoint `iterate`, result a list | **OOM (killed)** |
| (c) | thread the SAME list param unchanged through the fixpoint, result a list | compiles (11,971 B) |
| (d) | fresh `(list)` re-seed each round, but result consumed as an **Int64** (`List.len` inside) | compiles (633 B) |
| (f) | thread the list and GROW it with `List.push` each round, result a list | compiles (12,008 B) |

So the blowup is **neither the fixpoint loop itself, nor the fresh re-seed alone, nor the list result alone** —
it is the **conjunction**: a list-typed parameter re-seeded with a fresh `(list …)` (a value **not derived from
the incoming parameter**) **AND** the recursion's result consumed as a list. Threading the incoming list — even
mutating it by `List.push` every round (f) — compiles; re-seeding fresh while consuming the result as a scalar
(d) compiles. Only when both hold does the seed diverge.

**Why.** This is the same class as the fixed `eval_const` let-memoization blowup
([[eval-const-let-memoization-blowup]]) and the Tier-00 threaded-accumulator inference blowup
([[threaded-compound-accumulator-inference-blowup]]) — an inference/fold fixpoint that fails to reach a fixed
*kind* and re-expands. The likely mechanism (to be confirmed when the fix lands): when the parameter is
re-seeded with a literal `(list)` rather than threaded, the incoming value provides no kind constraint on that
argument position, so each recursive pass must re-derive the parameter's kind from the fresh literal; if the
result is *also* consumed as a list, the return-kind unification (the very back-propagation the single-pass fix
added) has to reconcile "fresh literal at the call site" against "heap result at the use site" on every
iteration, and the two-sided constraint re-triggers the inline/fold expansion instead of converging. When the
list is threaded (c/f) the incoming value pins the parameter's kind once, so the fixpoint closes; when the
result is a scalar (d) there is no return-kind constraint pulling the other way, so it closes too. The precise
mechanism matters less than the isolation: **the doc's one-variable description ("fresh re-seed") over-broadly
condemns a shape that actually compiles (d) and misses the second necessary condition (list result), which is
exactly the kind of imprecise trigger that sends a fix at the wrong half of the problem.**

**The requirement it drove.** The OOMing program cannot be a corpus case — it would hang the gate — so the
durable pin is the **passing side of the boundary**: corpus case *"a fixpoint loop that threads a growing list
accumulator returns that list"* (05-compound-types → 5), which proves threaded list accumulators in a fixpoint
are representable **today** and marks exactly where the frontier is. The open blowup is recorded in
`SPEC-BACKLOG` with the corrected trigger (the conjunction, with the four controls as the reduction), so a fix
targets both conditions rather than the doc's single one. General lesson, a recurrence of this loop's standing
rule: **a suspicious aggregate — here a handoff doc's one-line trigger — deserves a direct probe before it is
trusted; the probe here cost four `emit` runs and turned a one-variable claim into a two-variable conjunction,
which is the difference between a fix that works and a fix aimed at a shape (d) that was never broken.** And the
positive-frontier discipline: when the failing program can't be pinned (it OOMs), pin the nearest passing
neighbor and name the boundary — the corpus then guards the working path and localizes the gap.
