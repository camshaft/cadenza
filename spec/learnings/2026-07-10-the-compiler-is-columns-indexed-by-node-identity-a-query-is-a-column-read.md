# The compiler is columns indexed by node identity — a query is a column read, and the artifact is just the last column

*2026-07-10*

**What happened.** While capturing the invariants a from-scratch rebuild would need, the operator named the
organizing model he wants the compiler built around, and it is sharper and simpler than the incremental-
computation direction the prior IR-shape research had sketched. The prior learning
([[2026-07-10-the-pipeline-is-a-tree-above-and-a-flat-anf-core-below-and-ssa-is-a-property-not-a-fourth-ir]])
had, for the "stop re-deriving during inference" problem, recommended a node→type side table and floated a
query/demand-driven memoization "à la Salsa" (cache each derived result, track a dependency graph, recompute
the minimal affected set with early cutoff) as the incremental extension. The operator rejected the cache
explicitly and stated the model directly:

- **The compiler's whole state is columns indexed by node identity.** It is a set of `Vec`s (columns), each
  keyed by a `NodeId`, each holding one kind of fact — the resolved form, the solved type, the source
  position, the emitted artifact. A phase is not a tree transform; it is a producer that fills a column by
  reading the columns earlier phases filled. A downstream column reads an upstream one (following a node's
  origin identity) to recover what it needs — a span, a resolution — rather than the upstream phase forwarding
  it.
- **A fact is present or it is absent — `Some`/`None` — and that is the whole state of a slot.** No cache, no
  dependency graph, no invalidation. "What is the type of node N" is `types[N]` — a read.
- **A query is a one-off column read.** Because the query is *literally the read* the compiler already does to
  fill downstream columns, there is nothing to cache and nothing to keep in agreement.
- **Getting an artifact out is a query too.** The emitted bytes are the *terminal column*; a backend is the
  producer that fills it by reading the core and layout columns. "Give me the component bytes" is the same
  operation as "give me the type of node N" — a read of a column — differing only in which column.

The one hazard the operator and I pinned before making it normative: in a sparse fill-as-you-go model, `None`
must mean *only* "no answer determined here," never "the answer is negative." A decline, a rejection, or a
compile-provable poison is a *value* filled into the column at the decision point; a reader that needs a value
and finds `None` **declines rather than defaults**. Without that rule the sparse model silently miscompiles a
not-yet-filled slot as an absent-therefore-default value.

**Why.** This model makes three obligations that are otherwise hard-won either free or trivially true, which is
why it is the right foundation rather than a mere implementation taste:

1. **The queryable-oracle capability becomes free.**
   [tooling-and-lsp.md §The Compiler Is A Queryable Oracle](../capabilities/tooling-and-lsp.md) obliges the
   compiler to answer any static fact — a node's type, a name's resolution, the effect row — totally and
   *equal to what a full compile determines*. In a tree-walking compiler that means building a *second* query
   implementation that walks the same tree and must be kept in agreement with the compiler — the exact
   disagree-and-miscompile class the whole architecture fights. When the compiler's state *is* columns, the
   query is the read and there is no second implementation. The debugging affordance the operator wants —
   "give me back the type of this node id" — is the model's default behavior, not a feature bolted on.

2. **"Incremental equals batch" is true by construction.**
   [tooling-and-lsp.md §Incremental Equals Batch](../capabilities/tooling-and-lsp.md) obliges an incremental
   result to equal a batch result. If incrementality were a cache with invalidation, that equality would be a
   property to *prove* (and a stale cache is where it breaks). With no cache — incrementality is re-run at a
   coarse, module-level granularity and then read — the incremental answer *is* a batch answer, read from
   freshly filled columns. The correctness rests on there being no derived state to go stale, not on the
   completeness of an invalidation. This is why dropping the Salsa direction is a strengthening, not a
   simplification-at-a-cost: the cache was the thing that could be wrong.

3. **Emission stops being a privileged special case.** Making the artifact the terminal column means the
   deepest form of "emission serializes a lowered representation"
   ([reference-compiler.md §Emission Serializes A Lowered Representation](../capabilities/compiler-pipeline.md)):
   a producer that fills a column by reading earlier columns *structurally cannot* re-derive a decision an
   earlier column already holds. A second backend is then just a second producer of the artifact column over
   the same upstream columns — which is exactly the seam
   [[2026-07-10-the-implementation-design-directions-fold-into-the-architecture-records-everywhere-first]]
   fixed, now expressed in the columns model.

The model also subsumes, rather than sits beside, the storage decisions already made:
[intermediate-representations.md](../architecture/intermediate-representations.md)'s arena addressed by a
stable index *is* the column model's node identity, and its "solved type and source position in side tables
keyed by node identity" *are* the type and position columns. So this is not a new subsystem; it is the
recognition that the arena, the side tables, the query oracle, and the emitted artifact are one thing — columns
keyed by node identity — seen from four sides. The reason to make it normative now, ahead of a rebuild, is the
standing observation that a from-scratch authoring to a documented target is cheap while a refactor into this
shape is expensive: a compiler built tree-first and given queries later pays the second-implementation cost the
model exists to avoid ([overview §16](../overview.md); [constitution §XII](../../constitution.md)).

**The requirements it drove.** A new architecture document,
[query-engine.md](../architecture/query-engine.md), fixing: the compiler's state is columns keyed by node
identity, assigned deterministically from the program's structure; a phase fills columns by reading earlier
columns; every static fact is a column read; a query holds no cache and the compiler no dependency graph;
absence means only "no answer" while a decline/rejection/poison is a value in the column, and a reader that
requires a value and finds absence declines rather than defaults; provenance is recovered by a derived node's
back-reference to its origin, not forwarded; the artifact is the terminal column a backend fills; and
incrementality is coarse re-run, not invalidation. It also drove revisions elsewhere:

- [intermediate-representations.md](../architecture/intermediate-representations.md) — the "side tables keyed
  by node identity" section is recast as the type and position *columns* of the one model, and the
  demand-driven / Salsa framing is removed from its declared-default note.
- [reference-compiler.md](../architecture/reference-compiler.md) — three invariants a rebuild would otherwise
  rediscover as bugs were folded while capturing this: **the compiler is a deterministic function of its
  input** (no unordered-container iteration order or allocation address may reach a produced representation or
  the artifact — the compiler's *own* output is reproducible, distinct from the runtime determinism of what it
  compiles); **the evaluator bounds its own reduction and declines rather than diverges** (a non-terminating
  compile-time reduction is a clean decline, generalizing the unbounded-handler-context rule); and **a value
  built at compile time is indistinguishable from one built at run time** to equality, hashing, and keying (the
  const/runtime construction-agreement rule, grounded in
  [[2026-07-08-map-equality-miscompiles-across-the-const-runtime-construction-boundary]]).
- The prior IR-shape learning
  [[2026-07-10-the-pipeline-is-a-tree-above-and-a-flat-anf-core-below-and-ssa-is-a-property-not-a-fourth-ir]]
  carries a superseding note: its physical node→type side table stands; its Salsa caching extension is dropped
  for this cacheless model.

No corpus: this is compiler-internal architecture, invariant under the behavior oracle; the byte-identity
anchor is unaffected because the model changes how facts are *stored and read*, not what the emitted bytes are.
