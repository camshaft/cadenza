# Query-Engine Architecture

> **NORMATIVE — REFERENCE QUERY-ENGINE ARCHITECTURE.** This document prescribes the shape the reference
> compiler's state takes and the way every fact is read from it: that the compiler's state is a set of columns
> indexed by node identity, that a phase fills columns by reading earlier columns, that every static fact —
> the type of a node, a name's resolution, the effect row, and the emitted artifact alike — is a read of a
> column, and that an unfilled slot is the absence of an answer while a decline or rejection is a value.
> Its RFC-2119 requirements bind a compiler built to the Cadenza *reference architecture* and are citable by
> the requirement gate for such a compiler.
>
> **This document fixes the organizing model the other architecture documents assume.**
> [intermediate-representations.md](./intermediate-representations.md) fixes that a rung's nodes are held in
> an arena addressed by a stable index and that a node's solved type and source position are held beside the
> node; this document generalizes that into the single model — every pass's output is a column keyed by node
> identity — and fixes its consequences: what a query is, what absence means, and that the artifact is itself
> a column. It is the internal-architecture realization of
> [tooling-and-lsp.md §The Compiler Is A Queryable Oracle](../capabilities/tooling-and-lsp.md), which fixes
> the external obligation that an agent can ask the compiler for any static fact.
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying exactly one
> obligation, under a stable heading. Per [constitution §XIII](../../constitution.md), the requirements below
> name no concrete engine, prior prototype, incremental-computation framework, or source path; the descriptive
> lead-ins and the learning they cite carry the concrete grounding.

## Purpose And Scope

[tooling-and-lsp.md §The Compiler Is A Queryable Oracle](../capabilities/tooling-and-lsp.md) obliges the
compiler to answer any static fact about a program — a node's type, a name's resolution, the effect row —
totally, deterministically, and equal to what a full compile determines, and
[§Incremental Equals Batch](../capabilities/tooling-and-lsp.md) obliges an incremental result to equal a batch
result. Those obligations are trivially satisfiable, or hopelessly hard, depending on *how the compiler holds
its state*: a compiler whose facts live in a tree threaded through a recursive walk must build a *second*
query implementation that walks the same tree, and the two must be kept in agreement — the disagree-and-
miscompile class again. This document fixes the internal shape that makes the obligation free: the compiler's
state is a set of columns indexed by node identity, a fact is a read of a column, and the artifact is the last
column. There is then only one implementation — the columns and the passes that fill them — and a query, an
incremental result, and a batch compile are the same reads of the same columns.

The grounding is recorded in the learning
[the compiler is columns indexed by node identity, and every fact — including the artifact — is a column read](../learnings/2026-07-10-the-compiler-is-columns-indexed-by-node-identity-a-query-is-a-column-read.md).
It does not restate the tooling capability's external obligations
([tooling-and-lsp.md](../capabilities/tooling-and-lsp.md)) or the storage and reproducibility disciplines it
builds on ([intermediate-representations.md](./intermediate-representations.md),
[reference-compiler.md](./reference-compiler.md)); it fixes the model that unifies them.

## The Compiler's State Is Columns Indexed By Node Identity

The compiler does not hold its program as a tree of nodes each carrying its own fields; it holds a set of
columns, each a mapping from a node's identity to one kind of fact about that node — one column for the
resolved form, one for the solved type, one for the source position, one for the emitted artifact. A phase is
not a transform that rebuilds a tree; it is a producer that fills a column by reading the columns earlier
phases filled. This is the data-oriented realization of the arena-addressed storage
[intermediate-representations.md §A Rung's Nodes Are Held In A Contiguous Arena, Referenced By Index](./intermediate-representations.md)
already requires: the arena index *is* the node identity, and every per-node fact is a column keyed by it.

### Node Identity Is Assigned Deterministically From The Program's Structure

A node's identity MUST be assigned as a deterministic function of the program's structure, so that the same
input yields the same identities on every run and every column — and therefore every query answer and the
emitted artifact — is reproducible rather than dependent on allocation or traversal order
([constitution §II](../../constitution.md)).

### A Phase Fills Columns By Reading The Columns Earlier Phases Filled

Each phase MUST be a producer that reads one or more existing columns and fills one or more new columns keyed
by the same node identities, so that a phase's output is data attached to nodes rather than a rebuilt tree, and
the dependency of a later fact on an earlier one is the ordinary reading of an earlier column.

A column MUST be keyed by node identity and MUST be independently populated, so that a fact one phase
determines about a node is stored and read without disturbing any other fact about that node and a column that
a phase does not fill for a node is simply absent there (§Absence Is No-Answer).

## A Static Fact Is A Column Read

Because every fact the compiler determines is a filled column slot, answering "what is true of this node" is
reading that slot. There is no separate query machinery to build and keep in agreement with the compiler: the
query *is* the read. The same reads that a full compile performs to fill downstream columns are the reads a
tooling query performs to answer a question, which is why an incremental answer equals a batch answer by
construction rather than by a second implementation that must be verified to agree.

### Every Static Fact Is Answered By Reading A Column

A static fact about a program — a node's type, a name's resolution, the effect row at a point, the constraints
solved for a node — MUST be answered by reading the column that holds that fact, so that the compiler answers a
query by a lookup rather than by a separate analysis that re-derives the fact and could disagree with the
compile ([tooling-and-lsp.md §The Compiler Is A Queryable Oracle](../capabilities/tooling-and-lsp.md)).

### A Query Computes And Retains No Cache Of Its Own

A query MUST be a one-off read of the columns a compilation filled and MUST NOT maintain a memoization cache,
a dependency graph, or an invalidation protocol of its own, so that answering a fact is a read with no
retained derived state that a later change could leave stale (§Incrementality Is Re-Run, Not Invalidation).

### A Batch Compile And A Point Query Are The Same Reads Of The Same Columns

The reads a full compilation performs to fill its downstream columns and the reads a tooling query performs to
answer a question MUST be the same operation over the same columns, so that an incremental or point-query
answer equals what a full compilation determines because it is literally the same read, realizing
[tooling-and-lsp.md §Incremental Equals Batch](../capabilities/tooling-and-lsp.md) by construction rather than
by a parallel analysis kept in agreement.

## Absence Is No-Answer; A Decision Is A Filled Value

A column is sparse: a slot is either filled or it is not. The single hazard of a sparse, fill-as-you-go model
is conflating "no answer was determined here" with "the answer is a negative one." The model forbids that
conflation: an empty slot means *only* that no answer exists yet, and every negative outcome the compiler
decides — a decline, a rejection, a compile-provable trap — is a *value* filled into the column at the point
the decision is made, never represented by leaving the slot empty. This is the columns-model form of the
already-fixed disciplines that an undetermined type is a rejection rather than a default
([reference-compiler.md §An Undetermined Type Is A Rejection, Not A Default](./reference-compiler.md)) and that
a "no" is a first-class value produced where the decision is made
([reference-compiler.md §The Kind Of A "No" Is Fixed Where It Is Produced](./reference-compiler.md)).

### An Empty Column Slot Means Only That No Answer Was Determined

An absent slot in a column MUST mean only that the phase owning that column determined no answer for that node,
and MUST NOT carry any further meaning — not a default, not a negative outcome — so that absence is a single,
neutral state a reader handles explicitly rather than a value it may silently substitute for.

### A Decline, A Rejection, And A Poison Are Values In The Column

An outcome the compiler decides — a decline, a rejection with its diagnostic, or a compile-provable trap —
MUST be a value filled into the relevant column at the point the decision is made, so that a negative decision
is a fact carried like any other and is distinguished from the absence of a decision, consistent with a "no"
being a first-class value ([reference-compiler.md §A "No" Is A First-Class Value Produced Where The Decision Is Made](./reference-compiler.md)).

### A Reader That Requires A Value And Finds Absence Declines Rather Than Defaults

A phase that reads an upstream column and requires a value there MUST decline when the slot is absent rather
than substitute a default, so that a not-yet-determined fact can never be read as a convenient value and the
sparse model cannot silently miscompile a node whose answer was never filled
([reference-compiler.md §An Undetermined Type Is A Rejection, Not A Default](./reference-compiler.md)).

## Provenance Is Recovered By Back-Reference, Not Forwarded

A downstream phase frequently needs a fact an upstream phase holds — most often a node's source position, for a
diagnostic. It obtains that fact by carrying, on each node it produces, the identity of the node it was
produced from, and following that identity back to the upstream column that holds the fact. Provenance is
therefore read on demand from the phase that owns it, not copied into every intermediate node, so that a fact
lives in exactly one column and no phase widens its nodes to forward information it does not itself use.

### A Derived Node Carries The Identity Of The Node It Was Produced From

A node a phase derives from an earlier node MUST carry the earlier node's identity, so that a later phase can
recover any fact the earlier phases hold about the origin — its source position, its resolved form — by
following that identity back to the column that holds it.

### A Fact Is Read From The Column That Owns It, Not Forwarded Through Intervening Phases

A phase that needs an upstream fact MUST read it from the column that owns it, by following a node's origin
identity, rather than require every intervening phase to forward that fact through its own nodes, so that a
fact such as a source position is stored once in one column and an intervening representation is not widened to
carry information it does not consume.

## The Artifact Is The Terminal Column

The emitted artifact is not the product of a privileged final phase that stands outside the model; it is the
last column. A backend is the producer that fills the artifact column by reading the core and layout columns,
exactly as inference is the producer that fills the type column by reading the resolved column. "Give me the
component bytes" is therefore the same operation as "give me the type of this node" — a read of a column that a
producer filled — differing only in which column is read. This is the deepest form of emission serializing a
lowered representation ([reference-compiler.md §Emission Serializes A Lowered Representation](../capabilities/compiler-pipeline.md)):
because producing the artifact is filling a column by reading earlier ones, emission structurally cannot
re-derive a decision an earlier column already holds.

### Producing An Artifact Is A Column A Backend Fills, Not A Privileged Phase

A backend MUST produce the emitted artifact by filling a terminal column from the columns earlier phases filled
— the resolved core, the solved types, the boundary layout — rather than as a phase that stands outside the
columns model, so that emission is a producer reading earlier columns like any other and two backends are two
producers of the artifact column over one shared set of upstream columns
([backends-and-targets.md §A Backend Is A Function Of The Typed Core And A Target-Neutral Layout](./backends-and-targets.md)).

### The Artifact Is Obtained By The Same Read As Any Other Fact

Obtaining a program's emitted artifact MUST be a read of the terminal column, the same operation by which any
other static fact is obtained, so that building a program and querying a fact about it are one mechanism and a
tool that requests the artifact and a tool that requests a node's type do so through the same interface.

## Incrementality Is Re-Run, Not Invalidation

The model deliberately holds no derived state to invalidate. An incremental result is obtained by running the
producers again over the changed input and reading the columns they fill — not by tracking which prior results
a change invalidates. This is what keeps the model simple and correct-by-construction: there is no cache that
can be stale, because there is no cache. Responsiveness at scale comes from re-running at a coarse granularity
— recompiling the units that changed — rather than from fine-grained per-node dependency tracking, which the
compiler does not implement.

### The Compiler Holds No Dependency Graph And Invalidates No Cache

The compiler MUST NOT maintain a graph of which derived facts depend on which inputs, nor an invalidation
protocol over retained results, so that there is no derived state a change can leave stale and the correctness
of an answer does not rest on the completeness of an invalidation.

### An Incremental Result Is Produced By Re-Running, Not By Invalidating

An incremental result MUST be produced by re-running the producers over the changed input and reading the
columns they fill, and responsiveness MUST come from re-running at the granularity of the units that changed
rather than from fine-grained invalidation, so that an incremental answer is a fresh read equal to a batch
answer ([tooling-and-lsp.md §Incremental Equals Batch](../capabilities/tooling-and-lsp.md)) rather than a
reconstructed one that must be proven to match.

## Relationship To The Storage And Declared-Default Layers

This document fixes the *model* — state is columns keyed by node identity, a fact is a read, the artifact is
the terminal column, and there is no cache. [intermediate-representations.md](./intermediate-representations.md)
fixes the *storage* the model rides on: the arena addressed by a stable index (which is the node identity here),
the interning that makes a type column cheap, and the tree-above / flat-below shape of the represented program.
The *concrete choices* this model leaves open — whether a given column is a dense vector or a sparse map, the
encoding of a node identity, the granularity at which re-compilation is triggered — are declared-default facts
recorded at the declared-default location per [constitution §XIII](../../constitution.md), not fixed in a
requirement here.
