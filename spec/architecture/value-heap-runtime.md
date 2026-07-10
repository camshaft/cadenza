# Value-Heap Runtime Architecture

> **NORMATIVE — REFERENCE RUNTIME ARCHITECTURE.** This document prescribes the internal architecture of
> the **value-heap runtime** — the single, well-known component every derived program imports to
> construct and inspect its runtime values. Its RFC-2119 requirements bind a runtime built to this
> reference architecture and the compiler that emits programs against it; they are citable by the
> requirement gate for that compiler-and-runtime pair.
>
> **The runtime's interface identity and boundary behavior are already fixed by frozen contract** —
> [component-abi.md §The Value-Heap Runtime](../contracts/component-abi.md) makes the runtime a
> content-addressed well-known import that owns the value heap, holds no type tag or source name, and
> exposes only opaque handles. This document does not restate those ABI facts; it prescribes the
> *internal architecture* the contract deliberately leaves as "the runtime's private concern": the heap
> model, the operation interface as a calling contract, how collection representations realize their
> observable contracts, and how storage is reclaimed. Which concrete data structures realize each
> collection is a declared-default choice, recorded at the declared-default location per
> [constitution §XIII](../../constitution.md), not in a requirement here.
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying exactly
> one obligation, under a stable heading. Per [constitution §XIII](../../constitution.md), the
> requirements below name no concrete algorithm, engine, numeric width, prior prototype, or source path;
> the descriptive lead-ins and the learnings they cite carry the concrete grounding.

## Purpose And Scope

The compiler and the runtime are one versioned pair: the compiler emits programs that construct and
inspect their compound values by calling the runtime across a fixed import, and the runtime owns every
byte of those values' storage ([component-abi.md §The Runtime Owns The Value Heap And Its Representation](../contracts/component-abi.md)).
This document fixes *how the runtime is built* so that a from-scratch implementation reproduces the
representation-independence, the reclamation completeness, and the tag-free structural comparison the
language depends on, without rediscovering them. It realizes [overview §4](../overview.md) (determinism)
and [overview §8](../overview.md) (the component boundary), and it sits beside
[reference-compiler.md](./reference-compiler.md), which prescribes the compiler that emits against it.

The grounding is recorded in the learnings:
[the value-heap runtime is a shared component](../learnings/2026-07-05-the-value-heap-runtime-is-a-shared-component.md),
[the immutable heap is acyclic so reference counting is complete](../learnings/2026-07-04-immutable-heap-is-acyclic-so-reference-counting-is-complete.md),
[the runtime is tag-free — rendering walks a static shape](../learnings/2026-07-05-the-runtime-is-tag-free-rendering-walks-a-static-shape.md),
[persistent collections fit the tagless heap with no new machinery](../learnings/2026-07-05-persistent-collections-fit-the-tagless-heap-with-no-new-machinery.md),
[a list and a persistent vector are one type](../learnings/2026-07-06-a-list-and-a-persistent-vector-are-one-type-representation-is-the-runtimes-choice.md),
and [a keyed collection needs no serialization seam](../learnings/2026-07-06-a-keyed-collection-needs-no-serialization-seam-structural-comparison-is-tag-free.md).

## The Heap Model

Every runtime value is one uniform cell holding a reference count, a sequence of child handles, and a
raw byte payload — no per-value type tag and no source name. A scalar leaf has no children; a product
has one child per element; a sum carries a discriminant in its raw payload and its variant's value as a
child. This single shape is what lets the same operations serve tuples, records, sums, lists, maps, sets,
and byte sequences, and what lets the runtime's storage strategy change without any program observing it.

### The Heap Holds Structure And Data, Never A Type Or A Name

A runtime value MUST carry only its structure and its data — a product's elements, a sum's variant
discriminant, a leaf's payload — and MUST NOT carry a type identity, a field name, or a variant name, so
that the runtime is a name-free structural store and every source-level name is compile-time knowledge
the runtime never holds ([component-abi.md §The Runtime Does Not Name Or Render Values](../contracts/component-abi.md)).

The only runtime data that is not a structural element MUST be an array's element count and a sum's
variant discriminant, and neither MUST be a universal type tag a reader dispatches on, so that the
absence of type erasure — the compiler knows every value's static type at every use site — makes a
per-value type tag redundant.

### The Value Heap Is Acyclic So Local Reclamation Is Complete

The runtime MUST reclaim every value without a tracing collector, which is possible because the value heap
is acyclic: a strict, immutable language constructs no reference cycle, so the one class of garbage a
non-tracing discipline cannot reclaim is the one class the language cannot create
([memory-and-resource-model.md §The Value Heap Is Acyclic](../capabilities/memory-and-resource-model.md)).
The declared-default discipline that realizes this — a reference count with precise drop — is recorded at
the declared-default location, not fixed here.

Releasing a value's storage MUST NOT recurse in proportion to the value's depth, so that reclaiming a
deeply nested unique value cannot exhaust the runtime's own call stack — the release of a value's
children is driven by an explicit work list rather than by nested calls.

### One Product Primitive And One Sum Primitive

The runtime MUST provide one positional-array primitive that backs every product — a tuple and a record
alike — so that a record is a positional array whose slots are its fields in a canonical order and is
constructed and projected by the same operations as a tuple, with the field names supplied by the
compiler rather than stored.

The runtime MUST provide one tagged primitive that pairs a variant discriminant with a payload value, so
that every sum — including an optional, a result, and the abstract syntax tree — is one representation
the compiler assigns discriminants to in declaration order, and a nullary variant carries the unit value
as its payload rather than a distinct empty representation.

## The Operation Interface Is A Calling Contract

The operations the runtime exports form the fixed interface the compiler emits imports against. Their
identity and order are part of the ABI ([component-abi.md §The Value-Heap Runtime Crosses By A Well-Known Import](../contracts/component-abi.md));
the concrete vocabulary is recorded at the declared-default location. Beyond the interface's shape, the
operations carry an ownership contract — who consumes and who borrows a reference — that the compiler
must emit against, and a fixed split between an absent value and an out-of-bounds access.

### The Operation Set Is Index-Stable And Grows Only By Appending

The runtime's exported operations MUST retain a stable order across versions, and a new operation MUST be
added only by appending, so that the index the compiler bakes into a program's fixed component envelope
for each imported operation remains valid and adding an operation is a one-time envelope re-derivation
rather than a breaking renumbering.

### Constructors Consume And Accessors Borrow

An operation that constructs or derives a value MUST consume the references it is given — taking
ownership of each operand and producing a new owned value that leaves its operands' observable content
unchanged — so that persistence is the default and a caller that keeps an operand retains a reference
before passing it.

An operation that inspects a value MUST borrow — returning a child or a scalar without changing any
reference count — so that reading a value neither transfers nor releases ownership, and a caller that
lets an inspected child outlive its parent retains a reference before releasing the parent.

Before a value's storage is released, every child of it that the program will use again MUST have been
retained, so that releasing a parent can never leave a still-referenced child reclaimed — the ordering
obligation that makes the consume/borrow contract free of use-after-free.

### An Absent Value Is A Value; An Out-Of-Bounds Access Traps

An operation that looks a key up in a keyed collection, or advances an exhausted cursor, MUST return an
absence-of-value result rather than trap, so that a missing key or a finished traversal is data the
program folds rather than a halt.

An operation given an index outside the bounds of an otherwise-valid value MUST trap, so that an
out-of-bounds positional access is a fail-fast violation of a compiler-established invariant, kept
distinct from the absence-of-value that a key miss returns.

## Collection Representations Realize Observable Contracts

Each collection's *observable* contract — its ordering, its equality, its totality, its canonical byte
form — is fixed normatively elsewhere ([collections-and-text.md](../capabilities/collections-and-text.md),
[type-system.md](../capabilities/type-system.md)). The *representation* that realizes it is the runtime's
private choice, recorded at the declared-default location. The architectural obligation is that the choice
stay unobservable and that one invariant — canonicality — hold, because tag-free structural comparison
depends on it.

### A Collection's Representation Is Not Observable

The runtime MAY back a collection with any internal representation and MAY change it by size or usage, and
that choice MUST NOT be observable by any operation the executable semantics defines — including equality,
length, indexing, iteration order, and the value's canonical byte form — so that two collections with
equal contents are indistinguishable however each is stored
([collections-and-text.md](../capabilities/collections-and-text.md);
[memory-and-resource-model.md §Sharing Is Not Observable](../capabilities/memory-and-resource-model.md)).

An ordered sequence MUST be one type whose representation the runtime selects, so that a list and a
persistent indexed sequence are not two author-visible types and a sequence built by appending is
indistinguishable from one built by concatenation
([the list/persistent-vector learning](../learnings/2026-07-06-a-list-and-a-persistent-vector-are-one-type-representation-is-the-runtimes-choice.md)).

### Every Value Form Is Canonical So Structural Comparison Is A Value Comparison

Every value the runtime stores MUST have a canonical representation, so that comparing two values by
their tag-free structure is a correct comparison of the values they denote and a keyed collection needs
no separate serialization or comparison seam.

A representation that is not canonical on its own MUST be canonicalized before the value is used as a
collection key or compared, so that the one deferred-materialization form the runtime permits does not
break the structural-comparison invariant the whole keyed-collection story rests on
([the keyed-collection learning](../learnings/2026-07-06-a-keyed-collection-needs-no-serialization-seam-structural-comparison-is-tag-free.md)).

### Deferred Materialization Is Permitted Behind The Observable Bytes

The runtime MAY represent a derived value as an unmaterialized view of its operands and materialize it
lazily, and MUST make the view indistinguishable from the fully materialized value by every operation,
so that constructing a derived value cheaply is a representation choice the program never observes
([the rope-defers-materialization learning](../learnings/2026-07-05-a-bytes-rope-defers-materialization-behind-the-same-observable-bytes.md)).

## Storage Is Reclaimed Precisely And Reused When Unobservable

The runtime's reclamation is precise and source-determined, and it reuses storage in place when no
reference can observe the difference. The observable invariants — deterministic cleanup, unobservable
reuse and sharing — are already normative ([memory-and-resource-model.md](../capabilities/memory-and-resource-model.md));
the architectural obligation is that reuse be bounded and safe.

### Reclamation Is Precise And Source-Determined

The point at which a value's storage is released MUST be a deterministic function of the source, so that
reclamation timing is reproducible and carried by the runnable form rather than deferred to a collector
([memory-and-resource-model.md §Cleanup Is Source-Determined](../capabilities/memory-and-resource-model.md)).

### In-Place Reuse Fires Only At A Unique Reference

The runtime MAY refit a value's storage in place to build a new value instead of allocating, and MUST do
so only when the value being consumed holds a unique reference, so that reuse never mutates a value another
reference can observe and peak storage cannot grow — a reused cell was already live and about to be
released ([memory-and-resource-model.md §Reuse Is Not Observable](../capabilities/memory-and-resource-model.md)).

### A Small Value Need Not Occupy The Heap

The runtime MAY represent a value small enough to ride within a handle's own bits inline rather than in a
heap cell, and MUST make an inline value indistinguishable from a heap-resident one at every use site — a
fitting value is always inline so no inline-versus-heap twin of the same value can coexist, and every site
that dereferences a handle first admits the inline case — so that avoiding an allocation for a small value
is unobservable and never corrupts structural comparison.

## Relationship To The Normative And Declared-Default Layers

The runtime's *observable* behavior and *boundary* identity are fixed normatively:
[component-abi.md §The Value-Heap Runtime](../contracts/component-abi.md) (well-known content-addressed
import, owns the heap, tag-free, opaque handles, self-describing required-runtime address),
[memory-and-resource-model.md](../capabilities/memory-and-resource-model.md) (acyclic heap, source-determined
cleanup, reuse and sharing not observable), and [collections-and-text.md](../capabilities/collections-and-text.md)
(the list, map, and set observable contracts). This document adds only the construction architecture behind
them.

The runtime's *concrete choices* — the operation vocabulary and its indices, the reference-count-with-
precise-drop ownership discipline, the persistent-structure and byte-sequence representations, the
inline-handle encoding — are declared-default facts. The ownership discipline and the set representation
already have declared-default homes; the **operation vocabulary and its interface identity do not yet**,
though [component-abi.md](../contracts/component-abi.md) requires the interface identity to be "fixed at
the declared-default location." Recording that vocabulary at the declared-default location is the open
follow-up this architecture assumes.
