# Build Order — The Reproduction Plan

> **What this document is.** The order in which a from-scratch compiler built to the Cadenza reference
> architecture is assembled, and how each stage is validated before the next begins. It is the *reproduction
> plan*: the sibling architecture documents fix *what the compiler is* when finished; this one fixes *the
> sequence that gets there without a rewrite*, and for each stage states what must work, how to prove it
> works, and the recorded failures to avoid. It is **descriptive** — it carries no RFC-2119 requirements and
> is not cited by the requirement gate; it sequences and cites the normative documents that do. It exists
> because the gate obligations and the target architecture admit many build orders, and earlier generations
> established through the [learnings](../learnings/) that most orders force an expensive rewrite — a
> foundation retrofitted after the features that depend on it, a query layer bolted onto a tree, a kind
> classifier that has to become real inference. This is the order that does not.
>
> The concrete seed language, the crate layout, and the staging of the self-hosting generations are pinned at
> the declared-default location and in [bootstrap.md](../bootstrap.md); this plan is
> **generation-independent** — the seed reference compiler in a foreign language and the Cadenza-authored
> self-hosted compiler are both built in this order, against the same corpus oracle.

## The Two Forces And How The Order Reconciles Them

Two pulls act on the build, and a naive plan sacrifices one to the other:

- **Foundation-first.** The substrate a fresh implementation is most tempted to retrofit — the columns model
  ([query-engine.md](./query-engine.md)), records-everywhere resolution
  ([prelude-and-resolution.md](./prelude-and-resolution.md)), real inference solved once
  ([reference-compiler.md §Types Are Solved Once And Read Downstream](./reference-compiler.md)) — is exactly
  the substrate that is *ruinous* to add late, because every feature built on top of a stand-in accretes the
  special-casing the substrate exists to forbid. The predecessor's fused emit-walk could not be repaired
  because no rung owned one concern
  ([the nanopass ladder](../learnings/2026-07-09-the-compiler-is-a-nanopass-ladder-each-pass-an-exhaustive-match.md)),
  and its coarse kind-classifier had to be torn out and replaced with real inference
  ([the coarse-kind post-mortem](../learnings/2026-07-08-a-coarse-kind-classifier-re-derived-at-emit-is-the-wrong-inference-and-fails-one-way-at-every-lattice-point.md)).
  Those are the retrofits this order refuses.

- **Thin-slice-first.** Nothing is *validated* until an artifact runs and the gates read it. A plan that builds
  each phase to completion in isolation has no running artifact — and therefore no oracle — until the very end.

The reconciliation: **Stage 0 builds a thin vertical slice through every phase at once** — enough to compile
`(def (main) 42)` to a running component and stand up both gates — *on the real substrate*, not a stand-in.
Every later stage then **deepens one phase** against a gate that already exists. The foundation is built right
from the first slice; capability breadth grows through the gate. The order below is the deepening sequence.

## The Spine — Disciplines Held From Genesis Through Every Stage

These are not stages; they are established in Stage 0 and hold unbroken through all that follow. A stage is not
done if it breaks one.

- **The columns substrate.** The compiler's state is columns keyed by node identity; a phase fills columns by
  reading earlier ones; a query — including "emit the artifact" — is a column read
  ([query-engine.md](./query-engine.md)). Every stage adds or fills a column; none introduces a tree-walk that
  re-derives a fact a column holds. Build this first, because a query layer added to a tree-walking compiler is
  the second-implementation rewrite.

- **The two gates.** The behavior gate runs each corpus case and checks its recorded output
  ([conformance-gate.md](../capabilities/conformance-gate.md)); the differential gate runs the compiled
  artifact against an independent reference and classifies each case as agree, byte-differing-agree, decline,
  or disagree ([reference-compiler.md §Convergence Is Judged By Running The Artifact](./reference-compiler.md)).
  Both stand up in Stage 0 on a tiny corpus and grow with each stage. The signal is **zero disagreement**
  (soundness) read alongside a shrinking decline pile (coverage) — never a single "percent passing."

- **Decline, do not miscompile.** Outcomes are ordered by safety: a wrong value or a valid-but-trapping
  component is worse than a clean decline, which is worse than a correct compilation
  ([reference-compiler.md §Outcomes Are Ordered By Safety](./reference-compiler.md)). Every stage lands its new
  capability behind a decline: an unbuilt construct declines observably rather than emitting a plausible wrong
  artifact ([decline, do not miscompile](../learnings/2026-07-03-decline-do-not-miscompile.md)). When a partial
  build regresses, it moves toward decline, never back toward a wrong value.

- **The compiler is a deterministic function of its input.** No unordered-container iteration order or
  allocation address reaches a produced representation or the artifact
  ([reference-compiler.md §The Compiler Is A Deterministic Function Of Its Input](./reference-compiler.md)).
  This holds from the first slice, because the byte-identity gate is meaningless without it.

- **Spec-first.** The normative sentence and an executable corpus witness exist *before* the mechanism that
  satisfies them — the corpus is the source of truth and the compiler is its projection
  ([constitution §XII](../../constitution.md)). Building a mechanism ahead of the sentence that sanctions it is
  the recorded top process error; every stage below begins by pinning its corpus cases.

## The Stages

Each stage states what it **establishes**, what it **depends on** (and so why it cannot come earlier), which
normative requirements it **realizes**, how you know it is **done** (the concrete program, gate signal, or
query that proves it), and what to **watch out for** (the recorded failure it must avoid).

### Stage 0 — The Skeleton: Columns, A Thin Vertical Slice, And Both Gates

**Establishes.** The columns substrate and node-identity assignment; the reader (bytes → the AST column); a
trivial pass at every rung — resolve, infer, lower, select — each a real column producer, not a stand-in; one
backend emitting a scalar component; and both gates running on a handful of scalar corpus cases. The
decline-don't-miscompile discipline is wired from the first emit.

**Depends on.** Nothing above it — this is genesis. It depends only on the frozen contracts that predate the
compiler: the AST encoding, the component ABI, and the canonical value form
([contracts](../contracts/)).

**Realizes.** [query-engine.md](./query-engine.md) (the columns model end to end);
[reference-compiler.md §The Pipeline Is A Ladder Of Typed Intermediate Representations](./reference-compiler.md);
[compiler-pipeline.md §The Pipeline Has Defined Phases](../capabilities/compiler-pipeline.md);
[bootstrap.md §Ignition Demonstrates A Real End-To-End Derivation](../bootstrap.md).

**Done when.** `(def (main) 42)` compiles to a component that runs to `42`, and its bytes are validated
byte-identical to an independent encoder ([reference-compiler.md §Emission Is Validated Byte-Identical To An
Independent Encoder](./reference-compiler.md)); a second scalar case exercises a two-way branch (`if`); both
gates are green on the tiny corpus; and querying the type column for the literal node returns the scalar type.
The thinnest possible whole — reader to running artifact — is closed.

**Watch out for.** Fusing phases to reach the running artifact faster — the fused emit-walk is the
un-repairable shape ([the nanopass ladder](../learnings/2026-07-09-the-compiler-is-a-nanopass-ladder-each-pass-an-exhaustive-match.md));
keep one concern per rung even when each rung is trivial. Standing up the byte gate without a decline
discriminator — a decline stub byte-compared against real output reads as a false disagreement
([the byte gate conflates declines with miscompiles](../learnings/2026-07-07-the-byte-level-self-hosting-gate-runs-and-its-disagree-count-conflates-declines-with-miscompiles.md));
the gate must classify by *running* the artifact, not by inspecting its shape.

### Stage 1 — Records-Everywhere Resolution: The Foundation Of Compile-Time Meaning

**Establishes.** The prelude as one map of values; the two generic operations — one ordered lookup returning
the bound value verbatim, one generic projection that never inspects its key; the resolution modes
(value / key / pattern); and the meta channel by which a built-in type is a record carrying its meaning. Every
built-in — operator, collection constructor, type, sum, module — becomes a map entry.

**Depends on.** The reader and columns (Stage 0). It is otherwise the *earliest real foundation*, because it
fixes how a name acquires meaning, which every later stage reads. It comes before inference, patterns, effects,
and widths precisely so those land as prelude entries and meta fields rather than as resolver edits.

**Realizes.** [prelude-and-resolution.md](./prelude-and-resolution.md) in full;
[reference-compiler.md §Nothing Is Privileged By Name](./reference-compiler.md).

**Done when.** The discipline holds mechanically: the set of source-name spellings the resolver matches is the
fixed grammar set and does not grow when a built-in is added
([prelude-and-resolution.md §The Set Of Spellings The Resolver Matches Is The Fixed Grammar Set And Does Not
Grow](./prelude-and-resolution.md)) — a grep for name/field-string dispatch shows only the grammar keywords. A
`let`-bound `+` shadows the operator and a `def`-bound collection constructor shadows the builtin (the shadow
program runs correctly). Member access on a literal prelude record folds to the field value at resolve time.

**Watch out for.** The shadow miscompile — an operator or constructor arm that fires before the full lookup, so
a rebound name is silently ignored
([everything is a record, nothing is privileged by name](../learnings/2026-07-09-everything-is-a-record-nothing-built-in-is-privileged-by-name.md));
the fix is deletion of the special-case arm, not a new guard. A projection miss returning a default rather than
a decline — an absent field is a decline (built-in, open) or a rejection (user record, closed), never a
substituted value.

### Stage 2 — Real Inference, Solved Once Into The Type Column

**Establishes.** Full Hindley-Milner as a distinct pass — type variables, unification with occurs-check,
let-generalization, principal types — filling the type column, read by every pass below. Order-independence.
The bidirectional boundary where type-valued (generic) parameters are checked rather than unified.

**Depends on.** Resolution (Stage 1), because inference over a name needs to know what it denotes, and a bare
type name is reduced to the type it denotes by reading its meta channel at this pass
([prelude-and-resolution.md §The Meaning Of A Type Record Is Read At Its Use Site](./prelude-and-resolution.md)).
It must precede lowering, which reads the solved type rather than computing one.

**Realizes.** [reference-compiler.md §Types Are Solved Once And Read Downstream](./reference-compiler.md);
[type-system.md §Inference](../capabilities/type-system.md) and
[§Inference And First-Class Types Meet At A Bidirectional Boundary](../capabilities/type-system.md);
[intermediate-representations.md §A Node's Solved Type Is A Column Read By Node Identity](./intermediate-representations.md).

**Done when.** A forward reference, a self-reference, and a mutual recursion type to the same solution
regardless of visit order; an expression whose type is left undetermined causes a rejection, not a defaulted
type; and the type of *any* node is answerable by reading the type column
([tooling-and-lsp.md §The Compiler Is A Queryable Oracle](../capabilities/tooling-and-lsp.md)) — the first
place the debugging affordance ("what is the type of this node") becomes real.

**Watch out for.** Building a kind classifier as a stand-in for types — a coarse classifier re-derived at emit
is not inference and fails the same way at every lattice point
([the coarse-kind post-mortem](../learnings/2026-07-08-a-coarse-kind-classifier-re-derived-at-emit-is-the-wrong-inference-and-fails-one-way-at-every-lattice-point.md));
build real unification from the start. Re-deriving a type in any later pass — the whole point of solve-once is
that the type column is read, never recomputed.

**Approach within this stage — intrinsics before functions, and generic over the integer type.** The
implementation reaches full Hindley-Milner through a smaller first increment that is worth calling out because
it fixes the shape everything after it reuses: the *application* mechanism arrives first through **arithmetic
intrinsics**, not through user functions. `(+ a b)` is the application of a built-in operation value — the
identical mechanism `(f a b)` uses — so realizing it introduces application at its simplest, purely
compile-time-foldable case (no runtime closure, no capture), before the harder function cases (higher-order,
returned closures) that need β-reduction through the evaluator. Crucially, an arithmetic intrinsic is **generic
over the integer type**: `+` types at `(Int w) → (Int w) → (Int w)` for a single width variable `w`, so it
unifies its operands' widths and signedness rather than hard-coding `Int64` — the first real exercise of a type
variable and the same width-parametric machinery Stage 7 generalizes to every width. Getting a generic-over-width
`+` working end-to-end (fold `(+ 2 3)` to `5`, reject a width mismatch as a conflicting use) is the recommended
first foothold: it is the intersection of the prelude discipline (Stage 1 — the operation is a prelude value,
lowered at selection), real unification (this stage — one width variable), and the one evaluator (Stage 3 — the
constant folds). A built-in operation flows through the pipeline as a value and is translated to instructions
only at selection ([reference-compiler.md §A Built-In Operation Is A First-Class Value, Lowered At Selection](./reference-compiler.md));
resist the temptation to special-case an operator name in the resolver — it is a prelude entry reached by the
one lookup, exactly as a collection constructor or a type is.

### Stage 3 — The A-Normal Core And The One Compile-Time Evaluator

**Establishes.** Lowering of the typed representation to the A-normal core — every non-trivial subexpression
named — and the single reduction tier over it: constant folding, generic reduction, monomorphization,
specialization, and the elimination of the administrative bindings A-normalization introduces. Poison
collection and reachability; the erasure fence.

**Depends on.** The solved type column (Stage 2), because lowering reads types and monomorphization reduces
type-valued applications. The evaluator is built here because it is the one place compile-time meaning lives,
and both effects (Stage 6) and breadth-as-data (Stage 7) reduce *through* it — it must exist before them.

**Realizes.** [reference-compiler.md §The Core Representation Is In A-Normal Form](./reference-compiler.md),
[§Compile-Time Evaluation Is One Reduction Tier](./reference-compiler.md), and its trap / reachability /
meaning-preserving-rewrite subsections; [intermediate-representations.md](./intermediate-representations.md)
(the core names every intermediate value; single-assignment is a property of this core, not a fourth
representation).

**Done when.** The administrative bindings A-normalization introduces are eliminated so the core adds no
runtime cost — a folded `(+ 1 2)` is the constant `3`, and byte-identity holds *after* administrative-let
elimination; a compile-provable trap (`1/0`) fails the build with its diagnostic while the same trap shielded
by an untaken branch stays a runtime trap; the evaluator bounds its own reduction and declines rather than
hangs on a non-terminating reduction
([reference-compiler.md §The Evaluator Bounds Its Own Reduction And Declines Rather Than Diverges](./reference-compiler.md)).

**Watch out for.** Naive A-normalization that never eliminates its administrative redexes — the IR and the
emitted code bloat; elimination is the non-optional companion
([the core wants A-normal form](../learnings/2026-07-09-the-resolved-core-wants-anf-name-every-intermediate-so-perceus-and-effect-capture-are-precise.md)).
A fold that drops a dead branch also dropping that branch's type-check — value-preserving is not
rejection-preserving
([a fold that eliminates a branch must not eliminate its type-check](../learnings/2026-07-07-a-fold-that-eliminates-a-branch-must-not-eliminate-its-type-check.md)).

### Stage 4 — The Pattern Engine: One Set Of Probes And Binders

**Establishes.** The one match engine — an arm is a conjunction of probes and binders, a match a top-to-bottom
disjunction — with sum and product matching retrofitted onto it first for parity, then each further category
added as a new kind of probe. A binding position is a single-arm irrefutable match. The accept / reject /
decline triad at the decision point, and whole-pattern (nested) linearity.

**Depends on.** Resolution's pattern mode (Stage 1), the type column for exhaustiveness (variant counts, Stage
2), and the core to lower into (Stage 3). It is foundational for real programs but not for the thin slice, so
it deepens after the core exists.

**Realizes.** [reference-compiler.md §Matching Is One Engine Of Probes And Binders](./reference-compiler.md);
[prelude-and-resolution.md §A Pattern Name Binds Unless It Names A Constructor](./prelude-and-resolution.md).

**Done when.** The retrofit is at parity — existing sum/tuple cases compile byte-identical-or-better through
the one engine before any new category is added; a coverage defect (a non-exhaustive match) and a shape defect
(a wrong-arity or type-mismatched pattern) carry distinct machine-readable codes; a name bound twice anywhere
in a pattern, including a nested sub-pattern, is rejected; and a const-folded match decides at compile time
with no runtime ops (const-fold matching lands before the runtime path).

**Watch out for.** Compiling matches as a per-arm cascade rather than a shared engine — a new category then
becomes a parallel path that must agree with the first. Decision-tree code duplication — prefer a shared-prefix
trie with a linear fallback. First-match order broken by probe sharing — sharing a leading probe must never
reorder a later arm ahead of an earlier one. A binding-position pattern's refutable case reported with a
different code than the equivalent single-arm non-exhaustive match — the desugared and direct paths must agree
([exhaustiveness keys on the arm set, not the scrutinee value](../learnings/2026-07-07-exhaustiveness-hides-a-bug-in-the-static-scrutinee-present-arm-corner.md)).

### Stage 5 — One Backend To Completion, And The Value-Heap Runtime

**Establishes.** Instruction selection completed to the full value language, filling the artifact column; the
value-heap runtime the backend emits against — the tagless uniform cell, consume/borrow, canonical value
forms, inline handles, and the persistent collections; the component envelope; and the type-directed renderer.
This deepens Stage 0's trivial scalar backend into a complete one.

**Depends on.** The core and the evaluator (Stage 3), which the backend reads to fill the artifact column, and
the pattern engine (Stage 4), whose matches it emits. Compound *types* exist from Stage 2 and compound *const*
values from Stage 3; this stage adds their *runtime* construction and the runtime that holds them.

**Realizes.** [reference-compiler.md §Instruction Selection Emits Against A Fixed Runtime And Envelope](./reference-compiler.md)
and §The Reader, Printer, And Renderer Are Built As Duals; [value-heap-runtime.md](./value-heap-runtime.md) in
full; [backends-and-targets.md §A Backend Is A Function Of The Typed Core And A Target-Neutral Layout](./backends-and-targets.md)
(the backend is the producer of the artifact column); [query-engine.md §The Artifact Is The Terminal Column](./query-engine.md).

**Done when.** A program that constructs and inspects compound values — a record, a list, a map — runs
correctly and byte-identically to the independent encoder over a covering set; a program touching no compound
value emits against no runtime import; the runtime survives a deeply nested value under reclamation, hashing,
comparison, and materialization without exhausting its stack
([value-heap-runtime.md §The Value Heap Is Acyclic So Local Reclamation Is Complete](./value-heap-runtime.md));
and a value the evaluator folded at compile time compares and keys identically to the same value built at run
time ([reference-compiler.md §A Value Built At Compile Time Is Indistinguishable From One Built At Run Time](./reference-compiler.md)).

**Watch out for.** Treating the flat instruction rung as a shared pipeline stage rather than *this* backend's
representation — it is the linearizing backend's product, and a structured target would consume the core
directly ([backends-and-targets.md](./backends-and-targets.md)). A folded collection and a runtime-built
collection comparing unequal across the const/runtime boundary
([map equality miscompiles across the const/runtime construction boundary](../learnings/2026-07-08-map-equality-miscompiles-across-the-const-runtime-construction-boundary.md)).
A shared scratch-local mechanism over-reserving and breaking byte-identity for the client that needs less
([sharing the scratch-local mechanism cost right-shift its byte-identity](../learnings/2026-07-07-sharing-the-scratch-local-mechanism-cost-right-shift-its-byte-identity.md)).

### Stage 6 — Effects: Classify First, Resolve By Monomorphization

**Establishes.** Effects as records of their operations; classification of each handler arm by resumption shape
(tail-resumptive-once / never-resumes / general); tail-resumptive lowering to plain code with no continuation
object; the discharging handler as a compile-time constant resolved by inlining and per-context specialization;
the routing-agnostic declaration surface and the manifest as the computed union of delegated, reached effects.

**Depends on.** The core and evaluator (Stage 3) — effects lower *through* the core and reuse the
monomorphization machinery — and the completed backend (Stage 5) to emit the manifest boundary. It is last of
the mechanism stages because it is the deepest reuse of everything below it.

**Realizes.** [reference-compiler.md §Effects Are Classified First And Resolved By Monomorphization](./reference-compiler.md),
including the declaration/manifest and reified-continuation subsections;
[capabilities-and-effects.md](../capabilities/capabilities-and-effects.md).

**Done when.** A tail-resumptive handler lowers to plain code carrying no continuation object, no runtime
handler stack, and no evidence structure; the emitted manifest is exactly the union of the effects the program
delegates to the host and reaches from an entrypoint, and an effect discharged by a nearer in-program handler
is absent from it; a reified intra-program continuation does not span a host call; and a general (non-tail or
captured) continuation declines cleanly rather than emitting a valid-but-trapping component.

**Watch out for.** Misclassifying an arm — an arm not provably at-most-once-in-tail must be treated as the
general resuming class, the conservative direction
([compiling effect handlers, classify first](../learnings/2026-07-06-compiling-effect-handlers-classify-first-tail-resumptive-is-plain-code.md)).
A decline that leaks into a valid component that traps at run time — the most dangerous shape a decline can take
([a decline that leaks into a valid-but-trapping component](../learnings/2026-07-07-a-decline-that-leaks-into-a-valid-but-trapping-component-is-the-most-dangerous-shape.md)).
Recursive-perform state carried through shared mutable storage rather than threaded through the specialized
call boundary.

### Stage 7 — Breadth As Data: Widths, Collections, Numeric Completeness

**Establishes.** The remaining value-language breadth — integer widths, the full collection operation sets,
numeric completeness — added as prelude entries and meta fields rather than new machinery. An integer type is
`(Int width)`, a record whose meta channel carries its signedness and width; a new width is one prelude entry.

**Depends on.** Records-everywhere (Stage 1), so each addition is a map entry; the type column (Stage 2) for
the no-implicit-promotion rule; and the completed backend (Stage 5) to emit the width-selected machine
operations. It comes last because it is the *payoff* of the foundation — cheap because the substrate holds.

**Realizes.** [prelude-and-resolution.md §A Numeric Width Is A Type Record, Its Machine Operation Read From Its
Meta](./prelude-and-resolution.md); [numeric-model.md](../capabilities/numeric-model.md);
[collections-and-text.md](../capabilities/collections-and-text.md).

**Done when.** Adding a width is one prelude entry with no resolver edit (the Stage 1 discipline still holds);
two integer types unify only at equal width and signedness, and a mismatch requires an explicit conversion; the
signedness meta selects the machine operation (a signed versus an unsigned shift and comparison); and each
width's overflow traps at run time or is rejected when constant-provable, as the width-parametric
generalization of the checked-integer core.

**Watch out for.** A width smuggling in a resolver special-case — that is a violation of the Stage 1
discipline, not a way to add the width. An implicit promotion between widths — there is none; a mismatch is an
explicit-conversion site or a rejection.

## After The Stages — The Second Backend And The Self-Hosted Generation

The plan above builds *one* compiler, one backend, to the full value language plus effects. Two things follow
it, each a separate effort against the now-stable architecture rather than a stage of it:

- **A second backend** is added by plugging a new artifact-column producer into the seam
  ([backends-and-targets.md](./backends-and-targets.md)) — the entire front (Stages 1–4, 6–7) is shared
  unchanged, and the new backend inherits the front's decline boundaries and may widen them only where its
  target genuinely expresses more. This is why the seam is fixed early even though only one backend is built in
  the ordering: the second is bounded work, not a fork.

- **The self-hosted generation** re-authors this same compiler in Cadenza, in this same order, judged against
  the same corpus oracle and against the seed compiler by the differential gate
  ([bootstrap.md §The Self-Hosted Compiler Is Authored In Cadenza](../bootstrap.md)). The order is
  generation-independent; what changes is the authoring language, not the sequence or the disciplines.

## Grounding

This plan is the ordered reading of the architecture the [learnings](../learnings/) drove; the order itself is
recorded in
[the design directions fold into one architecture — records-everywhere is the foundation built first](../learnings/2026-07-10-the-implementation-design-directions-fold-into-the-architecture-records-everywhere-first.md)
and [the compiler is columns indexed by node identity](../learnings/2026-07-10-the-compiler-is-columns-indexed-by-node-identity-a-query-is-a-column-read.md),
which together establish that records-everywhere and the columns substrate are the foundation built first and
that everything else is a deepening on top of them.
