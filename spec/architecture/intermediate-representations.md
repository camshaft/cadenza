# Intermediate-Representation Shape Architecture

> **NORMATIVE — REFERENCE IR-SHAPE ARCHITECTURE.** This document prescribes the *representational shape* of
> the reference compiler's intermediate representations: which rungs are source-structured trees and which are
> flat sequences of named bindings, where single-static-assignment lives, how nodes are stored, and where a
> node's solved type and source position are held. Its RFC-2119 requirements bind a compiler built to the
> Cadenza *reference architecture* and are citable by the requirement gate for such a compiler.
>
> **This document sits beside [reference-compiler.md](./reference-compiler.md) and does not restate it.**
> That document fixes the *logical* pipeline — the nanopass ladder of typed sums, A-normal form at the core
> representation, and types solved once and read downstream. This document fixes the orthogonal *physical*
> axis that one leaves open: tree-versus-flat, single-static-assignment as a property rather than a
> representation, node storage, and where the type and position columns are keyed. The *columns model* those
> per-node facts are read through — that every fact, including the artifact, is a read of a column keyed by
> node identity — is fixed in [query-engine.md](./query-engine.md); this document fixes the storage those
> columns ride on. Where these touch, each cross-references rather than re-derives.
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying exactly one
> obligation, under a stable heading. Per [constitution §XIII](../../constitution.md), the requirements below
> name no concrete engine, prior prototype, or source path; the descriptive lead-ins and the learning they
> cite carry the concrete grounding.

## Purpose And Scope

A gate obligation fixes *what* a compiled program means, and [reference-compiler.md](./reference-compiler.md)
fixes the *logical* stages that produce it, but neither fixes the *shape* the stages are held in — and the
shape is where a from-scratch implementation, reaching for a representation that looks more powerful, most
often trades away the structure its earlier passes need or pays a locality and complexity cost for a benefit a
simpler layout would have delivered. This document fixes that shape so a fresh implementation reproduces it
directly rather than converging on it through an expensive rewrite. It realizes the same
[overview §7](../overview.md) and [overview §16](../overview.md) reproduction discipline the sibling documents
do: the compiler is a regenerable projection of the specification
([constitution §XII](../../constitution.md)), so the target shape is written down rather than rediscovered.

The grounding is a single learning that records the research and the precedent behind every requirement below:
[the pipeline is a tree above and a flat A-normal core below, and single-static-assignment is a property, not a
fourth representation](../learnings/2026-07-10-the-pipeline-is-a-tree-above-and-a-flat-anf-core-below-and-ssa-is-a-property-not-a-fourth-ir.md).
It confirms from external compiler precedent the two decisions this specification had already reached — A-normal
form at the core
([the core wants A-normal form](../learnings/2026-07-09-the-resolved-core-wants-anf-name-every-intermediate-so-perceus-and-effect-capture-are-precise.md);
[reference-compiler.md §The Core Representation Is In A-Normal Form](./reference-compiler.md)) and solve-once
([solve the type once, read it downstream](../learnings/2026-07-09-solve-the-type-once-read-it-downstream-never-re-derive.md);
[reference-compiler.md §Types Are Solved Once And Read Downstream](./reference-compiler.md)) — and adds the
representational axis they left open.

It does not restate the language semantics, the pipeline's logical stages, the effect-lowering strategy, or the
value-heap runtime; those remain in [capabilities](../capabilities/), [reference-compiler.md](./reference-compiler.md),
and [value-heap-runtime.md](./value-heap-runtime.md).

## The Representation Is A Tree Above The Core And Flat At And Below It

The pipeline has two regimes divided by the A-normal core. Above the core, the passes are name resolution,
type inference, pattern-match and exhaustiveness checking, desugaring, and diagnostics — each of which reasons
about the *source's nesting and structure*, so their representations keep that structure as a tree. At and
below the core, the passes are the one compile-time evaluator, precise reclamation and in-place reuse,
continuation capture for effects, and instruction selection — each of which needs *value flow made explicit and
every intermediate named*, which a nested tree hides. A-normal form is the hinge: it is where the
source-structured tree becomes a linear sequence of named bindings, and — because a named-binding core with
join points is already single-static-assignment — it is also where single-assignment's benefits arrive without
a graph rewrite.

### A Rung Above The Core Is A Source-Structured Tree

An intermediate representation above the A-normal core MUST retain the source's nesting as a tree in which a
subexpression is a child of the expression that contains it, so that the passes that reason about source
structure — name resolution, type inference, pattern-match and exhaustiveness checking, and desugaring — read
that structure directly rather than reconstruct it from a flattened form.

A rung above the core MUST NOT be linearized into single-assignment named bindings, so that the nesting a
pattern's arms, a scrutinee's shape, and a diagnostic's span are expressed in is not discarded before the
passes that consume it have run.

### The Core Names Every Intermediate Value

The A-normal core representation MUST name every non-trivial computation as a binding whose operands are each
a name or a constant, so that the passes at and below the core read an explicit value flow rather than one
implicit in the nesting of an expression, even though the core retains structured control — its conditionals,
matches, and bindings — rather than a flattened block graph
([reference-compiler.md §The Core Representation Is In A-Normal Form](./reference-compiler.md)).

A pass at or below the core that must know a value's last use or the values live at a program point MUST read
that from the core's named bindings rather than reconstruct it from a nested expression, so that reclamation
and continuation-capture consume the explicit value flow the naming provides.

### The Fully-Linearized Block Form Is A Linearizing Backend's Representation

The reduction of the core to a flat sequence of blocks reached by explicit transfers MUST be performed by a
backend whose target is a linear instruction stream, and MUST NOT be a rung the core is obligated to pass
through before a backend whose target has structured control flow consumes it, so that the basic-block form
is a linearizing backend's representation of the core rather than a shared stage every target descends through
([backends-and-targets.md §The Flat Instruction Rung Is A Property Of A Linearizing Backend](./backends-and-targets.md)).

## Single-Static-Assignment Is A Property Of The Core, Not A Fourth Representation

An A-normal (or continuation-passing) core whose control-flow join points take parameters is *born*
single-static-assignment: a block is a function, a join point's parameter is what a merge-point pseudo-value's
left side would name, and the value passed to a join point is what its argument would carry. The correspondence
and its provenance are recorded in the grounding learning. The consequence is that single-assignment is not a distinct
representation the compiler builds by a construction pass; it is a property the core already has by being
A-normal with join points. This is why the reference compiler builds neither a sea-of-nodes graph nor a
phi-placement pass — both would re-derive, at a locality and complexity cost, a property the core carries for
free.

### The Single-Assignment Property Is Obtained By A-Normalization, Not By A Construction Pass

The compiler MUST obtain the single-static-assignment property by lowering to the A-normal core with
parameterized join points rather than by a separate pass that inserts merge-point pseudo-instructions into a
prior representation, so that value flow is single-assignment because every intermediate is a named binding
rather than because a construction pass made it so.

A control-flow merge MUST be expressed as a parameterized join point whose parameters are the values live
across the merge, so that a value flowing from two predecessors is an argument passed at each transfer rather
than a merge-point pseudo-instruction reconstructed over a flattened graph.

### The Compiler Builds No Whole-Program Value-Dependence Graph As Its Representation

The compiler MUST NOT represent a function body as a graph in which control and data dependence are edges
among floating value-producing nodes to be scheduled into an order, and MUST instead carry an explicit ordered
sequence of named bindings, so that the representation the compiler stores, serializes, and debugs is a linear
form with good locality rather than a scattered node graph that degrades memory layout as lowering adds nodes.

## Node Storage Is An Arena Addressed By A Stable Index

The recursion-and-locality symptoms a nested tree of heap-boxed nodes exhibits — poor cache locality from
pointer chasing, and native-stack exhaustion on a deeply nested value — are addressed by *storage*, not by
changing the representation's logical shape. Holding a rung's nodes in a contiguous arena addressed by an
integer index, and traversing or releasing a deep structure by an explicit work list rather than by nested
calls, delivers the locality and the recursion bound to *every* rung, including the source-structured trees
above the core. This is the same release-without-recursion discipline the runtime is already held to
([value-heap-runtime.md §The Value Heap Is Acyclic So Local Reclamation Is Complete](./value-heap-runtime.md)),
applied to the compiler's own representations.

### A Rung's Nodes Are Held In A Contiguous Arena, Referenced By Index

Each intermediate representation MUST hold its nodes in a contiguous arena in which a node references another
node by a stable index into that arena rather than by an owning pointer to a separately-allocated node, so that
a rung is a compact indexable region with predictable locality rather than a graph of pointer-chased
allocations.

An index a node carries to another node MUST remain valid for the lifetime of the rung it indexes into, so
that a pass reads a referenced node by indexing rather than by following a pointer whose target may have moved.

### No Pass Traverses Or Releases A Representation By Unbounded Native Recursion

A pass that walks or releases an intermediate representation MUST bound its own stack use independently of the
depth of the program it processes — driving the walk by an explicit work list rather than by a native call per
level of nesting — so that a deeply nested program is processed in stack proportional to a constant rather than
to its nesting depth, consistent with [reference-compiler.md §Compilation Cost Is Bounded In Nesting Depth](./reference-compiler.md).

## The Solved Type And The Source Position Are Columns Keyed By Node Identity

Solve-once requires that inference materialize each node's type and that no later pass re-derive it
([reference-compiler.md §Types Are Solved Once And Read Downstream](./reference-compiler.md)); this document
fixes *where* that materialized type lives. It lives in a column keyed by the node's identity — one of the
per-node columns that are the compiler's whole state
([query-engine.md §The Compiler's State Is Columns Indexed By Node Identity](./query-engine.md)) — so that
reading a node's type is a column read rather than a re-inference, and type equality over interned types is an
identity comparison. The node's source position is a column too, keyed by the same identity, so that position
lives in exactly one place and a downstream phase recovers it by following a node's origin identity back to it
rather than by every intervening representation forwarding it
([query-engine.md §Provenance Is Recovered By Back-Reference, Not Forwarded](./query-engine.md)).

### A Node's Solved Type Is A Column Read By Node Identity

The type a node was solved to MUST be held in a column keyed by the node's identity and read from that column
by every pass below inference, so that a machine-representation or lowering decision reads the solved type by a
lookup rather than by re-deriving it (§Types Are Solved Once And Read Downstream under
[reference-compiler.md](./reference-compiler.md)).

### Types Are Interned So Equality Is Identity And Structure Is Shared

A type the compiler holds MUST be interned so that two structurally-equal types are one shared value with one
identity, so that comparing two types is an identity comparison and a repeated structural type is stored once
rather than copied at every node that carries it.

### A Node's Source Position Is A Column, Not A Field Of The Node

A node's source position MUST be held in a column keyed by the node's identity rather than stored inside the
node, so that the position a diagnostic reports is read from the one column that owns it — by following a
derived node's origin identity when the position belongs to an earlier phase's node — without widening every
node to carry a fact it does not itself consume.

## Relationship To The Logical-Pipeline And Declared-Default Layers

This document's requirements are the *shape* companion to the *logical* requirements in
[reference-compiler.md](./reference-compiler.md): the nanopass ladder (§The Nanopass Ladder), A-normal form at
the core (§The Core Representation Is In A-Normal Form), and solve-once (§Types Are Solved Once And Read
Downstream) are fixed there; the tree-above / flat-below division, single-assignment-as-a-property, arena
storage, and the type-and-position side tables are fixed here. Neither narrows what a conforming compiler is —
a compiler that passes both gates by other means still conforms ([reference-compiler.md](./reference-compiler.md),
[conformance-gate.md](../capabilities/conformance-gate.md)); this document exists because the gate obligations
admit many shapes and this is the one that survives contact with the whole language.

The *concrete choices* this shape leaves open — the arena's element encoding, the interning structure, and
whether a given per-node column is a dense vector or a sparse map — are declared-default facts recorded at the
declared-default location per [constitution §XIII](../../constitution.md), not fixed in a requirement here. The
columns model that reads these representations — that every fact, including the emitted artifact, is a column
read, and that the compiler holds no cache to invalidate — is fixed in
[query-engine.md](./query-engine.md).
