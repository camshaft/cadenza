# Set Collection — Choice: persistent-ordered-set

> **The default choice for the `set-collection` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins a `Set` reached through the `Set`
> prelude record, realized by the same persistent-structure runtime as `Map` with the value column
> dropped.

## The choice

A `Set` is `Map`-without-values: the same element (key) discipline, the same determinism, one fewer
column. Its operations are reached as member access into the `Set` prelude record, exactly as
`Map.insert` is `(. Map insert)` and `Bytes.of` is `(. Bytes of)`:

| Operation | Shape | Meaning |
|---|---|---|
| `Set.empty` | `(Set.empty)` | the empty set |
| `Set.of` | `(Set.of <list>)` | the set of a list's elements, duplicates collapsed |
| `Set.insert` | `(Set.insert <set> <elem>)` | a new set with `<elem>` added (a no-op value if already present) |
| `Set.remove` | `(Set.remove <set> <elem>)` | a new set with `<elem>` absent (a no-op value if already absent) |
| `Set.contains` | `(Set.contains <set> <elem>)` | `Bool` — total membership, never traps |
| `Set.len` | `(Set.len <set>)` | the number of elements |
| `Set.union` | `(Set.union <a> <b>)` | the set of elements in either |
| `Set.intersection` | `(Set.intersection <a> <b>)` | the set of elements in both |
| `Set.difference` | `(Set.difference <a> <b>)` | the set of elements in `<a>` not in `<b>` |
| `=` | `(= <a> <b>)` | structural equality — true exactly when the elements are equal, order-independent |

`Set.insert`/`remove`/`union`/`intersection`/`difference` each return a **new** set — the value heap is
immutable and acyclic (memory-and-resource-model.md; [[immutable-heap-is-acyclic-so-reference-counting-is-complete]]),
so a set is a persistent value with structural sharing, never mutated in place. `(Set.of (list …))` is
the canonical written form of a set literal, the way `(Bytes.of (list …))` is a byte sequence's; there
is no separate `(set …)` literal keyword, so the corpus writes sets through `Set.of` over a list.

## No positional access — the one place Set, Map, and List differ

`List` has fallible positional access (`List.at → Option`); `Map` and `Set` have total key/element
membership (`Map.get → Option`, `Set.contains → Bool`) and **no** positional access, because they are
unordered — there is no "element 0" of a set (collections-and-text.md §"Set Membership Is Total"). This
is deliberate: offering `Set.at` would expose the internal iteration order as if it were part of the
value, which it is not. A program that wants the elements in order calls an iteration/`to-list`
operation, which yields them in the deterministic element-derived order (below), not a positional
accessor.

## Determinism is inherited from the element order, not invented

A set's canonical byte form and its iteration order both place elements in the fixed order derived from
the elements (deterministic-value-form.md §"Ordering Of Aggregate Members Is Fixed" — a set is an
unordered aggregate; collections-and-text.md §"Set Iteration Is Deterministic"). That order is exactly
the total order the element type offers through three-way `compare` (core-semantics.md §"A Total Order
Is Observed Through A Three-Way Comparison"), so a set of ints sorts numerically, a set of strings
lexicographically over scalars, and a set built in any insertion order has one byte form. This is why a
`Set` is realized by the same **ordered** persistent structure as `Map` rather than a hash set whose
bucket order depends on a seed: the order must be a pure function of the elements for the determinism
guarantee to hold.

## The element constraint

A `Set T`'s element type `T` must have a canonical byte form and a total order — the two properties the
dedup and the deterministic order need. This is checked as a compile-time predicate over the element
type-value at instantiation (type-system.md §"A Generic Constraint Is A Compile-Time Predicate Over
Type-Values"): an ordered scalar (`Int`/`UInt`/`BigInt`/`Rational`/`Bool`/`Char`), a `String`, a
`Symbol`, or a structural composite of such, satisfies it; a `Function` element (no canonical form) is
rejected with the existing unsatisfied-constraint diagnostic. No new diagnostic code — a set over an
un-orderable element is the same compile-time constraint rejection a generic instantiation over a
failing type argument already carries.

## A set's elements are runtime data, not part of its type

Two sets of the same element type are the **same type** regardless of which elements they hold —
exactly as two maps with different key sets are the same `Map` type (spec/semantics/05-compound-
types.sexp §"two maps with different keys are unequal, not a type error"). So comparing two sets with
different elements is well-typed and yields `false` (they do not contain the same elements), never a
shape-mismatch type error. This is the crucial counterpoint the map cases already pin, carried onto the
set path: a set's elements are a runtime collection, not a fixed shape like a record's field set or a
tuple's arity.

## Realization / gating

`Set` is a later-generation value form; its corpus cases carry `(needs collections)` (the tag the map
cases use) and the seed's behavior gate skips them until a generation realizes the persistent-set
runtime — the same CHAMP-family structure the map runtime targets ([[rc-heap-persistent-ds-sota-2026-07-05]]),
with the value column dropped. Until then a generation that does not build sets **declines** rather
than miscompiles ([[2026-07-03-decline-do-not-miscompile]]).

## Resolved forks

- **Primitive, not `Map<T, Unit>`.** A set rides the deterministic-value-form machinery directly rather
  than laundering it through a map whose unit values are noise the equality and serialization paths must
  ignore. `Set` is `Map` minus the value column — one runtime family, not two.
- **Ordered persistent structure, not a hash set.** The element order must be a pure function of the
  elements (for determinism), so the realization is the same ordered persistent structure as `Map`, not
  a seeded hash set whose iteration order varies.
- **No positional accessor.** Membership is `contains → Bool`; ordering is exposed only through
  deterministic iteration, never a `Set.at` that would leak internal order as a value.
