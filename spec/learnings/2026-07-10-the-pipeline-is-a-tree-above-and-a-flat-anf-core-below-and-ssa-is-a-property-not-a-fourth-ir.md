# The pipeline is a source-structured tree above and a flat A-normal core below — and SSA is a property of that core, not a fourth IR to build

*2026-07-10*

**What happened.** A deep, multi-source research pass into compiler intermediate-representation design was run
to settle a recurring pull toward "make the IR fancier sooner": should the native reference compiler `rcdzc`
(then a nominal `Ast → Hir → Mir → Lir` pipeline, every rung a recursively-nested tree) adopt a sea-of-nodes
graph, or single-static-assignment (SSA), or a flat instruction list *early* — and specifically should the
mid-level `Mir` be SSA, and should the high-level `Hir` be SSA? The intuition driving the question was sound in
its symptoms: a deeply recursive tree is walked over and over, deep nesting risks stack exhaustion, pointer-
chased nodes have poor cache locality, and tree-walking Hindley-Milner inference feels like it re-derives the
same type at every visit. The research (real-compiler precedent, adversarially fact-checked) gave a consistent
answer that is *not* "make everything SSA/flat as soon as possible," and it lined up cleanly with two decisions
this specification had already made independently — A-normal form at the core
([[2026-07-09-the-resolved-core-wants-anf-name-every-intermediate-so-perceus-and-effect-capture-are-precise]])
and solve-the-type-once
([[2026-07-09-solve-the-type-once-read-it-downstream-never-re-derive]]) — while adding a new axis neither had
fixed: the *physical shape* of each rung.

The five findings, each with its precedent and its caveat:

1. **Sea of nodes is the road not taken.** V8 spent roughly a decade on a sea-of-nodes optimizer (Turbofan) and
   left it for a flat control-flow-graph IR (Turboshaft), reporting ~3× more L1 data-cache misses under
   sea-of-nodes (up to 7× in some phases), optimizer compile time cut in half by the switch, one phase (load
   elimination) up to 190× faster, and a whole class of bugs from hand-managing separate *effect chains* and
   *control chains* — a mix-up whose failure surfaced months later. **The honest caveat:** this is a single
   vendor's data (the blog calls its own 5%-of-compile-time figure "handwavy," and the 2× win is the optimizer
   tier, not the whole pipeline), and the premise that "HotSpot C2 regretted sea-of-nodes" is *false* — C2 and
   GraalVM still ship it successfully. The regret is V8/JIT-specific and partly blames Turbofan's
   implementation, not the idea. So the lesson is not "sea-of-nodes is bad" but "it buys an ahead-of-time
   Hindley-Milner-plus-effects compiler nothing it needs, at a real locality and complexity cost" — decline it.

2. **SSA belongs at a mid-level IR, never at a source-close high-level one.** Two production compilers make
   opposite SSA choices yet agree on placement. Rust keeps its high-level IR (`HIR`) a source-structured
   **tree** where type-checking happens, and its mid-level IR (`MIR`) **flat** — a control-flow graph of basic
   blocks, each a list of statements and one terminator, with *no nested expressions* (`x = a + b + c` becomes
   temporaries) — but deliberately **not SSA**: it computes which *subset* of locals happen to be
   single-assignment as an analysis over a non-SSA IR, so SSA is a queryable property of some locals, not a
   whole-IR invariant. Swift lowers its type-checked syntax tree to `SIL`, a flat mid-level graph that **is**
   full SSA, using basic-block *arguments* in place of phi-nodes — and even Swift does not have SSA at first
   lowering: "raw SIL" is not fully SSA; SSA-construction produces "canonical SIL" a pass later. The cost of
   putting SSA at the *high* level is what both avoid: it destroys the nesting a pattern-match compiler,
   desugarer, exhaustiveness checker, and diagnostic need.

3. **Flat scales, but flatness is two independent wins that get conflated.** Production mid-level IRs are flat
   basic-block graphs, not expression trees, and that is the form register allocation and dataflow passes want;
   the sea-of-nodes-versus-CFG data is the concrete (if single-vendor) evidence for the locality and
   pass-iteration gains. But "flat" bundles two orthogonal things: **(a)** *storage layout* — nodes held in an
   arena addressed by integer index rather than pointer-linked heap boxes, which is what actually buys cache
   locality and removes unbounded native recursion, and which a *tree* can adopt without ceasing to be a tree;
   and **(b)** *single-assignment linearization* — naming every intermediate so value flow is explicit, which
   is a semantic property (ANF/SSA), independent of layout. The intuition "a flat list beats a recursive tree"
   is right about **(a)** at every rung and right about **(b)** only below the level where source structure
   still does work.

4. **A-normal form with join points already *is* SSA — so SSA is never a fourth IR to construct.** Appel's "SSA
   is Functional Programming" establishes that an SSA control-flow graph and a set of mutually-recursive
   functions are the same object in two notations: blocks are functions, a phi-node's left side is a function's
   formal parameter, and its right-side arguments are the actual arguments at each call. Consequence for a
   functional language: an A-normal (or continuation-passing) core with join-point parameters is *born*
   single-assignment, delivering SSA's dataflow benefits with **no separate phi-placement / SSA-construction
   pass**. Effects lower *through* this core — Koka compiles handlers by a monadic translation into plain typed
   lambda calculus (every effectful expression sequenced through a bind that makes the continuation an ordinary
   captured-and-resumed function argument), and OCaml's Flambda2 middle-end is a continuation-passing-style IR.
   **Caveats:** continuation-passing style is strictly *more* general than SSA (it can encode `call/cc`), so the
   equivalence is with SSA's well-behaved subset; "no construction pass" is true for *conversion* (the core is
   born single-assignment) but choosing *minimal* join-point parameter lists is the same analysis as minimal
   phi-placement; and naive let-ANF can need O(n²) re-normalization when optimizations interleave with
   re-normalization (Kennedy 2007), which the join-point / block-argument form (the Swift-SIL, MLton shape)
   avoids — so prefer that form over naive let-nesting.

5. **Inference does not re-derive if you elaborate once into a node-keyed side table, and intern the types.**
   The fix for "tree-walking HM repeats work" is not flattening; it is materializing the solved type per node
   (a table keyed by node identity that every later pass reads) and interning/hash-consing types so equality is
   identity and structural types are shared — which this specification had already fixed as solve-once
   ([[2026-07-09-solve-the-type-once-read-it-downstream-never-re-derive]],
   [reference-compiler.md §Types Are Solved Once And Read Downstream](../architecture/reference-compiler.md)),
   grounded in the coarse-kind post-mortem
   ([[2026-07-08-a-coarse-kind-classifier-re-derived-at-emit-is-the-wrong-inference-and-fails-one-way-at-every-lattice-point]]).
   The research adds the *physical* realization the requirement leaves open — a table keyed by node identity,
   over interned types — and the incremental extension (a query/demand-driven memoization à la Salsa: cache each
   derived result, record the dependency graph, recompute only the minimal affected set on change, with an
   "early cutoff" that stops propagation when a changed input yields an unchanged result). Constraint-based or
   effects-based HM does not change the answer — collect constraints, solve once, write the solution into the
   side table — it only makes "solve once" the more valuable. The one caveat worth carrying: early cutoff at the
   syntax-tree level holds only if positions are *not* stored inside the tree nodes, so a span-carrying tree
   defeats it — position belongs in a side table too.

   > **Superseded on the incrementality question (2026-07-10, same day).** The "query/demand-driven
   > memoization à la Salsa" extension sketched above was *not* adopted. The operator's decision was the
   > simpler and stronger model recorded in
   > [[2026-07-10-the-compiler-is-columns-indexed-by-node-identity-a-query-is-a-column-read]]: the compiler's
   > whole state is columns keyed by node identity, a query (including "give me the artifact") is a one-off
   > *column read* with **no cache, no dependency graph, and no invalidation**, and incrementality is coarse
   > re-run rather than early-cutoff. The "side table keyed by node identity" this finding calls for is exactly
   > that model's type and position columns — so the *physical realization* here stands; only the caching
   > extension is dropped. The position-out-of-nodes caveat remains right, now for the reason that a fact lives
   > in exactly one column.

**Why.** The unifying reason the answer is a *gradient* rather than "SSA/flat everywhere ASAP" is that the two
halves of the pipeline serve opposite masters. Above the core, the consumers are name resolution, type
inference, pattern-match and exhaustiveness checking, desugaring, and diagnostics — every one of which needs the
*source's nesting and structure* intact, because that structure is the thing they reason about; flattening or
single-assigning there throws away their input. Below the core, the consumers are the compile-time evaluator,
precise-reclamation and in-place-reuse analysis, continuation-capture for effects, and instruction selection —
every one of which needs *value flow made explicit and each intermediate named*, which is exactly what a
recursively-nested tree hides. A-normal form is the hinge between the two regimes: it is where the
source-structured tree becomes a linear sequence of named bindings, and — because a named-binding core with join
points is already SSA — it is *also* where SSA's benefits arrive, for free, without a graph rewrite. The
recursion-and-locality symptoms that motivated the question are real but are addressed by a storage decision
(arena-of-nodes addressed by integer index, with explicit work-lists instead of native recursion — the same
release-without-recursion discipline the runtime already fixes,
[value-heap-runtime.md §The Value Heap Is Acyclic So Local Reclamation Is Complete](../architecture/value-heap-runtime.md)),
which is orthogonal to SSA and applies to *every* rung including the trees. Conflating the storage win with the
SSA win is what tempts a compiler toward sea-of-nodes or early-SSA, paying the structure-loss and complexity
cost for a locality benefit that a flat *layout* would have delivered on its own.

The reproduction-critical framing: the answer to "should `Mir` be SSA?" is **it need not be classical
phi-node SSA — if the core is A-normal with join points it already is SSA, and you never write an SSA-
construction pass**; and the answer to "should `Hir` be SSA?" is **no — it stays a source-structured tree, and
the only thing it should borrow from the flat world is arena-and-index storage**. This matters as durable
documentation because refactoring an existing tree-of-boxes pipeline into this shape is expensive, while
authoring a fresh pipeline *to* this shape is cheap — so the shape belongs written down as the target, not
rediscovered by a rewrite ([overview §16](../overview.md); the compiler is a regenerable projection of the
specification, [constitution §XII](../../constitution.md)).

**The requirement it drove.** A new focused architecture sibling,
[intermediate-representations.md](../architecture/intermediate-representations.md), which prescribes the
*representational shape* axis the existing [reference-compiler.md](../architecture/reference-compiler.md)
leaves open: that the rungs above the A-normal core are source-structured trees and the core and rungs below it
are flat sequences of named bindings; that single-static-assignment is a property the A-normal core already
carries rather than a distinct representation reached by a construction pass, so no sea-of-nodes graph and no
phi-placement pass are built; that every rung is stored as an arena of nodes addressed by a stable index rather
than a pointer-linked tree, and no pass releases or traverses a representation by unbounded native recursion;
and that the solved type and the source position are held in side tables keyed by node identity over interned
types, not carried inside the nodes. It cross-references rather than restates the ANF, solve-once, and
nanopass-ladder requirements already in [reference-compiler.md](../architecture/reference-compiler.md), which
this learning confirms from external precedent rather than revises.
