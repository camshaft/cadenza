# Set is a primitive collection, the sibling of Map — not a Map of Unit

*2026-07-05*

**What happened.** The collection vocabulary gains a third built-in beside `List` and `Map`: a
**`Set`** — an unordered collection whose elements are of one type and each present at most once.
It is a *primitive* collection with its own canonical byte form and boundary representation, not a
userland `Map<T, Unit>` and not a `List` the program dedups by hand. The operator confirmed `Set` as
a genuine gap while reviewing the primitive-type inventory: the language could model a set only as a
`Map` carrying junk unit values (wrong equality, wrong byte form) or a `List` that loses the
uniqueness and order-independence invariants the moment it is built.

**Why the language wants it.** A compiler — the self-hosting workload the whole type inventory is
shaped around — leans on sets constantly: the free-variable set of an expression, the visited/seen
set of a graph walk, the declared capability set of a module, the set of names already bound in a
scope. The specification's own prose is written in sets: a record's fields "are a **SET**", a sum's
"variant **SET**", exhaustiveness is checked "against the scrutinee sum type's variant **set**". Those
are compile-time meta-level sets today, but the compiler that manipulates them at run time needs a
first-class value with set semantics, and modeling one as a `Map` with dummy values is exactly the
kind of representation lie ([[the-runtime-is-tag-free-rendering-walks-a-static-shape]]) the language
avoids: the junk values leak into equality and into the canonical byte form.

**Why it must be primitive, not `Map<T, Unit>`.** A `Set` is not sugar over a `Map` for the same
reason a `Map` is not sugar over a `List` of pairs: the *invariants and the canonical form* are the
point. `Map` already rides the deterministic-value-form machinery — its keys serialize in a fixed
key-derived order (deterministic-value-form.md §"Ordering Of Aggregate Members Is Fixed";
collections-and-text.md §"Map Iteration Is Deterministic"), independent of insertion order, so two
maps built in different orders are one value with one byte form. A `Set` needs the *identical*
treatment over its elements, and the cleanest way to get it is to reuse that machinery directly rather
than launder it through a `Map<T, Unit>` whose unit values are noise the equality and serialization
paths must then learn to ignore. `Set` is `Map`-without-values: same key discipline, same determinism,
one less column.

**The semantics (mirror Map, drop the value):**
- A `Set` associates **elements** of one type; it contains each element at most once
  (collections-and-text.md §"A Map Associates Keys With Values", element-only analogue).
- Two sets are equal exactly when they contain equal elements, **independent of insertion order** —
  the set analogue of order-independent map equality, already witnessed for `Map` in
  `05-compound-types.sexp`.
- Iterating a set visits its elements in a **deterministic order derived from the elements**, agreeing
  with the order its canonical byte form places them in (the same rule `Map` iteration follows).
- **Membership is total, not indexing.** `Set.contains : (Set T, T) → Bool` is a total predicate; a
  set is unordered, so it offers **no** `Set.at` positional access (unlike `List`). This is the one
  place the `List`/`Map`/`Set` shapes genuinely differ: `List` has fallible positional access, `Map`
  and `Set` have total key/element membership, none of them trap.
- Set-algebra operations — `Set.union`, `Set.intersection`, `Set.difference` — plus `Set.insert` /
  `Set.remove` / `Set.len`, each returning a new set (the heap is immutable and acyclic;
  [[immutable-heap-is-acyclic-so-reference-counting-is-complete]] — persistent structural sharing, no
  mutation).

**The element constraint (why it is not free).** A `Set`'s elements must have a **canonical byte form
and a total order**, exactly as a `Map`'s keys must — that is what makes deduplication and the
deterministic element order well-defined. This is expressible with the existing generic-constraint
machinery (type-system.md §"A Generic Constraint Is A Compile-Time Predicate Over Type-Values"): the
element type-value is checked against an "orderable / has-canonical-form" predicate at instantiation,
and a `Function` element (no canonical form) is rejected with the existing unsatisfied-constraint
diagnostic — no new mechanism, and it pairs naturally with the three-way `compare`/`Ordering`
primitive ([[ordering-is-a-prelude-sum-three-way-compare]]) that the canonical element order is
defined in terms of.

**Boundary and contract impact — additive.** The type-mapping table gains one row: `Set T → list<T'>`
in canonical (element-sorted) order — precisely how a `Map`'s keys already serialize, so the boundary
learns nothing new about ordering. Defining a canonical byte form for a value that previously had none
is an **additive** change under deterministic-value-form.md §"Additive Evolution" and
component-abi.md's additive-evolution clause: **no contract version increment**. It touches the same
frozen contracts `Map` already touches, in the same way.

**Realization / gating.** `Set` is a later-generation value form, not the seed's — its corpus cases
carry `(needs collections)` (the same tag `Map` cases use) and the seed's behavior gate skips them
until a generation realizes the persistent-set runtime (CHAMP-style, the same family the map runtime
targets — [[rc-heap-persistent-ds-sota-2026-07-05]]). The recorded oracle is the set semantics above;
a generation that does not yet build sets **declines** rather than miscompiles
([[2026-07-03-decline-do-not-miscompile]]).

**The requirements it drove.** [collections-and-text.md](../capabilities/collections-and-text.md)
gains a "Sets" section mirroring "Maps": §"A Set Is A Collection Of Unique Elements" (one element
type, each element at most once, order-independent equality), §"Set Membership Is Total" (a total
`contains` predicate, no positional access), and §"Set Iteration Is Deterministic" (element-derived
order agreeing with the canonical byte form). The type-mapping default gains the `Set T → list<T'>`
row. Corpus witness: a new `19-sets.sexp` mirroring the map cases in `05-compound-types.sexp`
(construction, order-independent equality, membership, dedup, the empty set, and — the crucial
counterpoint the map cases already pin — that two sets of the same element type are the *same type*
regardless of their elements, since a set's elements are runtime data, not part of its type).
