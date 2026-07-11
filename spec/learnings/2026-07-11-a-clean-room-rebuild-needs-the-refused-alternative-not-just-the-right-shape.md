# A clean-room rebuild needs the refused alternative recorded, not just the right shape

*2026-07-11*

**What happened.** With the architecture documents mature enough to describe the compiler's finished shape, the
operator posed the real test: *if you had only the design docs and not the source, could you rebuild the
compiler faithfully — or would you repeat the mistakes?* Auditing the current columns-rewrite source against the
architecture documents answered it precisely. The documents get the **shape** right almost everywhere — the
nanopass ladder, the columns model, records-everywhere with the meta channel, solve-once, the one evaluator, the
pattern engine, the boundary-is-the-signature, decline-don't-miscompile, the backend seam. A clean-room build
would reproduce all of that. But at several points the document states the *correct answer* without recording
the *tempting wrong answer it refuses* — and a from-scratch build reaches for the wrong answer first, precisely
because it is locally reasonable. The gaps clustered where a mechanism had a plausible-but-wrong first
implementation the operator had already rejected once:

1. **Bounding the evaluator's recursion.** The document requires the compiler to *bound the evaluator's own
   reduction so a non-terminating reduction declines rather than hangs*, but says nothing about *how*. A
   from-scratch build reaches for one of two mechanisms, both of which the implementation tried and rejected:
   a **runtime depth bound** (a counter that declines past N reductions) explodes exponentially on a *branching*
   recursive body — a tree reader spawns several self-calls per level and floods the reduction long before any
   depth limit is meaningful; and a **body-on-the-reduction-stack set** (decline if the body being reduced is
   already active) *false-positives* on legitimate non-recursive nesting like `(f (f v))`, where the same body
   nests twice but both calls terminate. The mechanism that is actually sound is a **static call-graph DFS**:
   detect recursion as a structural property of the resolved call graph, computed *without reducing* (walk the
   body's callee edges, following a reference to its lambda, and report recursion iff the body is reachable from
   itself), gated at the single β-reduction choke point every application funnels through (`eval.rs:284`, gated
   at `eval.rs:223`; both refused alternatives are recorded as comments at `db.rs:60` and `db.rs:129` with
   regression tests at `tests.rs:2032` and `tests.rs:2066`). Without the refused alternatives written down, a
   rebuild re-derives the depth bound or the on-stack set and rediscovers their failure modes.

2. **Why a type constructor is a native intrinsic and not a closure.** The document says a parametric type is
   *applied through its meta channel* — correct — but a build that has just implemented compile-time closures
   (β-reduction of `(fn …)` applications) will reasonably ask "why not make `List`, `(Int N)`, and `->` ordinary
   lambdas over type values?" The answer, learned by trying it: β-reduction *cannot assemble a type value*,
   because substitution treats a type value as an inert leaf — there is no reduction rule that turns
   `(fn (w) …) 8` into the *type* `Int8`; only a native operation can construct a type from its arguments. So a
   type constructor bottoms out on an **intrinsic** riding the ordinary `apply(intrinsic)` fold path
   (`Prim::IntCtor`/`UIntCtor`/`FnCtor`, no `TypeCtor` node exists — the fold and the annotation site share one
   builder so they cannot drift, `eval.rs:525`/`571`), *not* on a lambda. This is the difference between
   "everything is a value applied generically" (true) and "everything is a lambda" (false) — the meta channel
   dispatches to an intrinsic, and the intrinsic is the principled bottom.

3. **Type annotation is a dedicated node, not a lambda.** For the same reason, `(: e T)` cannot be a prelude
   lambda `(fn (e t) e)`: that lambda discards its second argument, whereas annotation must turn the *value* of
   its second argument into the *type constraint* on its first — a distinct node the type pass unifies
   (`Resolved::Annot` at `resolved.rs:259`, unified at `infer.rs:106`, erased at `lower.rs:51`). A rebuild that
   models annotation as a lambda silently loses the constraint.

4. **Compile-time β-reduction is capture-safe by closed arguments, not by α-renaming.** A rebuild that reaches
   for hygienic substitution will add α-renaming on every β-reduce — real machinery, but *unnecessary here* and
   worth not building: the arguments substituted at compile time are *closed* (they resolve in the caller's
   scope and are copied with fresh occurrences whose parent is the copied structure), so capture cannot occur
   (`eval.rs:168`). The document should record that the hygiene argument is closedness, so a rebuild neither
   omits hygiene unsafely nor over-builds α-renaming for a case that does not need it.

The audit also flagged the inverse hazard, which is why the fold below is conservative: several mechanisms
described in *implementation-dated* design docs and in episodic memory — a `Core::Let` A-normalization step, the
compound collection constructors, a `CDZ0305` erasure-fence code that descends into compound slots — are **not
present in the current columns-rewrite source** (they are on an unmerged branch or belong to the superseded
pre-rewrite compiler). Folding those into normative architecture would write requirements for code that does not
exist, violating [constitution §XV](../../constitution.md) (a requirement binds to a mechanism that detects its
violation on a line). The reproduction documents must track *what is verified present*, not what memory or a
design doc anticipates.

**Why.** A clean-room reproduction document has a different job from a description of a finished system. A
description can state the right answer and stop; a reproduction guide must also *close off the wrong turns*,
because the builder will independently generate the same locally-reasonable wrong answers the original did — a
depth bound for recursion, a lambda for a type constructor, a fixed field for signedness — and rediscover their
failure modes at the same cost. The architecture documents already do this well in the places where a failure
was dramatic and early (the fused emit-walk, the coarse-kind classifier); the gaps are where the *right* answer
was reached through a quieter pivot the document then recorded only in its final form. The rule this yields:
**a reproduction requirement that has a tempting wrong alternative must name the alternative and why it fails**,
either in the requirement's descriptive lead-in or in the build order's watch-outs — a "rather than X" that
names a real refused mechanism, not just a generic caution. Otherwise the specification is a faithful photograph
of the destination that still lets a traveler take every wrong road to reach it.

**The requirements it drove.** Additions to the reproduction documents (naming no engine per
[constitution §XIII](../../constitution.md); grounding here):

- [reference-compiler.md §The Evaluator Bounds Its Own Reduction](../architecture/reference-compiler.md) — the
  bound is a static property of the call graph computed without reducing, and the descriptive lead-in names the
  two refused alternatives (a depth bound, an on-stack set) and why each fails.
- [reference-compiler.md §Nothing Is Privileged By Name](../architecture/reference-compiler.md) /
  [prelude-and-resolution.md](../architecture/prelude-and-resolution.md) — a type constructor bottoms out on an
  intrinsic because a compile-time reduction cannot assemble a type value from a lambda; the annotation is a
  dedicated node because a lambda discards the argument annotation must read as a type.
- [build-order.md](../architecture/build-order.md) — the current honest status recorded: runtime user functions
  and recursion are not yet built, every user call folds, and all user recursion declines — so a rebuild knows
  the compile-time-reduction tier stands alone until the runtime-call stage, rather than assuming a `Core::Call`
  it will not find.
