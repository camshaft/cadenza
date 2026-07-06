# Decision — Set Collection

**The decision.** The surface and representation of Cadenza's `Set` — an unordered collection of
unique elements of one type, the third built-in collection beside `List` and `Map`. The
collections-and-text.md capability fixes the *semantics* a set must have (unique elements of one type,
order-independent equality, total membership, deterministic element-derived iteration order agreeing
with the canonical byte form). This decision pins the concrete *operation surface* an author writes
and the runtime representation that realizes those semantics.

**Why the language wants it.** A self-hosting compiler leans on sets constantly — the free-variable
set of an expression, the visited set of a graph walk, the declared-capability set of a module, the
set of names already bound in a scope. The specification's own prose is written in sets (a record's
fields "are a SET", a sum's "variant SET"). Modeling a set as a `Map<T, Unit>` (junk unit values that
leak into equality and the canonical byte form) or a hand-deduped `List` (loses the uniqueness and
order-independence invariants) is a representation lie the language avoids. See
`spec/learnings/2026-07-05-set-is-a-primitive-collection-not-a-map-of-unit.md`.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- A set contains elements of one type, each at most once (collections-and-text.md §"A Set Is A
  Collection Of Unique Elements").
- Two sets are equal exactly when they contain equal elements, independent of insertion order
  (collections-and-text.md §"A Set Is A Collection Of Unique Elements").
- Membership is total — a `contains` predicate that never traps — and a set offers **no** positional
  access, because it is unordered (collections-and-text.md §"Set Membership Is Total").
- Set iteration visits elements in a deterministic element-derived order that agrees with the
  canonical byte form (collections-and-text.md §"Set Iteration Is Deterministic";
  deterministic-value-form.md §"Ordering Of Aggregate Members Is Fixed").
- The element type must have a canonical byte form and a total order (what makes dedup and the
  deterministic order well-defined), enforced as a compile-time predicate over the element type-value
  (type-system.md §"A Generic Constraint Is A Compile-Time Predicate Over Type-Values"), reusing the
  three-way `compare`/`Ordering` order (core-semantics.md §"A Total Order Is Observed Through A
  Three-Way Comparison").
- A `Set T` has one boundary representation — `list<T'>` in canonical element-sorted order
  (options/type-mapping/component-model-types.md). Adding it is an **additive** change (a value that
  previously had no boundary or canonical form), permitted without a contract version increment by
  deterministic-value-form.md §"Additive Evolution" and component-abi.md's additive-evolution clause.
- The set's storage is accountable against the deterministic resource measure and lives on the
  immutable acyclic value heap (memory-and-resource-model.md; the persistent-structure family the map
  runtime already targets).

## Choices

- [`persistent-ordered-set`](./persistent-ordered-set.md) — a `Set` reached as member access into the
  `Set` prelude record (`Set.empty`, `Set.of`, `Set.insert`, `Set.remove`, `Set.contains`, `Set.len`,
  `Set.union`, `Set.intersection`, `Set.difference`), realized by the same persistent-structure runtime
  as `Map` with the value column dropped, keyed on the element's canonical order. **The default.**

DEFAULT: persistent-ordered-set
