# `match` on user sums is the last major emit frontier — self-hosting is now an emit-coverage checklist, not a blocker

*2026-07-07*

**What happened.** After the multi-argument-call arc closed
([[2026-07-07-n-ary-calls-wired-end-to-end-the-round-trip-becomes-the-feature]]), a bookkeeping cycle
(the spike only updated docs — marking the fixed tiers, refreshing `compiler.cdz`'s header) prompted
taking stock of the actual remaining distance to *compiler-compiles-compiler*. Measuring the compiler's
own source against its emit surface makes the gap concrete and finite:
- `compiler.cdz` uses **~41 `match` expressions over 11 user sum types** (`Node`, `Core`, `Instr`,
  `Prim`, `IList`, `Def`, `FList`, …) — the spine of every pass — but its **emit-side `Core` has no
  `KMatch`**, no user-sum type declaration, and no user-sum construction. It emits arithmetic,
  comparisons, `if`, `let`, locals, and N-ary calls; it does not yet emit a program that *declares and
  matches its own sum type*.
- It uses **~19 `String.*` / `b"…"` operations** — the emit path builds Bytes (its output) but the
  source also compares and slices strings/bytes.
- It is **pervasively recursive** — compiling it walks deep, where the bounded wasm stack bites (TCO,
  the standing scale item).

Crucially, each of these is a **subset-frontier item, not a seed gap**: the *seed* compiles a user-sum
`match` fine (verified — a `Color` `match` → 31), so the language has the capability; it is the
Cadenza-authored compiler's *emit path* that must grow to produce code for a program that uses it.

**Why.** This is the shape of the endgame, and naming it is the value. For most of the arc the blocker
was a *seed* defect — a shape the seed miscompiled or declined, found by probing and pinned as a corpus
case that flipped green when fixed. Those are exhausted. What remains is a different kind of work:
**growing the compiler's emit coverage until it contains its own source** — a checklist of constructs
(`match` on user sums being by far the largest, since it is how every pass is written), not a hunt for
bugs. The distinction matters for how the loop should read the remaining cycles: a seed defect earns a
corpus case; an emit-coverage item earns a *scope entry* (what the emit path can vs. can't produce), and
its "fix" is a feature the compiler gains, whose regression guard is the compiler's own source
compiling once it lands. The self-hosting milestone is now bounded and countable: **when the emit path
covers user-sum `match` + construction, string/bytes comparison, and survives deep recursion, the
compiler's own source is within its accepted subset and `compiler-compiles-compiler` closes.** There is
no unknown blocker left — only known coverage to fill.

The `match`-on-user-sums item is worth singling out because of the leverage: it is not one feature but
*the* feature the compiler is written in. Every stage (`resolve`, `fold`, `lower`, `serialize`, `read`,
the reader's node dispatch) is a `match` over a user sum. So the emit path gaining user-sum declaration
+ construction + `match` is the single largest step toward self-inclusion — after which the source's
remaining needs (strings, recursion/scale) are comparatively narrow. It is the emit-side dual of the
reader's node-dispatch work: the reader learned to *decode* a tagged node and dispatch on its variant;
the emit path must learn to *produce* code for a program that declares such nodes and dispatches on them.

**The requirement it drove.** No new corpus case — the behavior (user-sum `match`) is already richly
pinned as a *seed* capability (05-compound-types' recursive sum-match, expression-tree evaluator,
nested-payload-binder cases all exercise it), and the compiler's *emit* of it is not yet a shape to pin
(it declines/is-absent, and a "compiler emits a match" case is only meaningful once the emit path
attempts it). The durable output is **SPEC-BACKLOG item 20** — the self-inclusion frontier as a coverage
inventory (emit `match` on user sums + construction; string/bytes comparison on the emit path; TCO for
deep recursion), priority-ordered — plus this learning framing the endgame as a countable checklist
rather than an open blocker. It consolidates the reframing
([[2026-07-07-self-hosting-gate-shifts-from-seed-capability-to-bootstrapping-subset]]) into the concrete
remaining items, so the operator sees the distance at a glance and the loop knows to pin each emit
capability as it lands (a feature gained), not to hunt for a next defect (there isn't one).
