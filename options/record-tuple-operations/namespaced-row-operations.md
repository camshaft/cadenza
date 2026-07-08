# Record And Tuple Operations — Choice: namespaced-row-operations

> **The default choice for the `record-tuple-operations` decision** (see [README.md](./README.md) for
> the decision and the requirements a choice must satisfy). It pins the concrete forms an author writes
> to reshape a record or a tuple, over the existing closed record and tuple value model, completing the
> explicit-`project` operation the rows learning promised
> (`spec/learnings/2026-07-04-records-are-rows-open-by-default.md`).

## The choice

Reshaping is reached as **member access into a `Record` (or `Tuple`) prelude record**, exactly as
collection operations are reached through `Set` and `List` (`Set.insert`, `List.at`). Because a field
name and a tuple position are **static** operands — a label the compiler resolves, not a runtime value
(the `.` accessor takes its key as a label, `tuple.N` its index as a literal) — these are **special
forms** the compiler recognizes by head, under a prelude-record prefix. They are not ordinary functions
over runtime values: a field-name list is written literally, the way `(record (a 1))` writes field
names, not passed as a `List<Symbol>`.

Every operation **yields a new value** and never mutates its operands (the value heap is immutable and
acyclic), and every operation's result shape is **fixed statically** from the operands' shapes — so the
emitted component carries a concrete closed record shape or tuple arity and the operation introduces no
runtime-determined field set or length (type-system.md §"A Record Row Is Reshaped Only Through An
Explicit Operation Yielding A New Value").

## Record primitives

Three primitives express every record reshape. Each is a row operation: `project` restricts, `without`
reduces, `merge` combines.

| Form | Yields | Rejects |
|---|---|---|
| `(Record.project r (a b …))` | a record whose fields are exactly `a b …`, each bound to the value `r` holds for it | `CDZ0212` if any named field is absent from `r` |
| `(Record.without r (a b …))` | a record holding `r`'s fields **except** `a b …` | `CDZ0212` if any named field is absent from `r` |
| `(Record.merge r s)` | a record whose field set is the union of `r`'s and `s`'s, each field bound to its source record's value | `CDZ0211` if `r` and `s` share any field name |

`Record.merge` is **strict and unbiased**: it never chooses a winner for a shared field, because a
record's fields are a *set* and cannot hold one name twice — the same reason `(record (a 1) (a 2))` is
rejected (`CDZ0201`, core-semantics.md §"A Record Has A Fixed Set Of Named Fields"). Overwriting a field
is therefore never silent; a program that means to replace a field says so with `Record.with` below.

The field-name list `(a b …)` is written literally in the operand position — a bare list of names, the
same names a `record` literal or a `.` access writes — not a runtime value. Projecting or dropping the
empty set is well-formed: `(Record.project r ())` names no field (its result is the record with no
fields, whose canonical form the value-form contract fixes) and `(Record.without r ())` is `r`
unchanged.

## Record derived operations

The remaining record operations are **defined by a meaning-preserving rewrite** to the primitives — the
compiler MAY lower them directly, but each denotes exactly its rewrite, so the primitives are the whole
semantics (the same posture the trait layer takes: an implicit convenience "provably rewrites to the
explicit passing … without changing emitted bytes", type-system.md §"Ad-Hoc Polymorphism Is An
Explicitly Passed Dictionary").

| Form | Meaning (its rewrite) | Rejects |
|---|---|---|
| `(Record.extend r (z v))` | `(Record.merge r (record (z v)))` — **add** an absent field `z` = `v` | `CDZ0211` if `z` is already present in `r` |
| `(Record.with r (z v))` | `(Record.merge (Record.without r (z)) (record (z v)))` — **update** a present field `z` to `v`, possibly at a new type | `CDZ0212` if `z` is absent from `r` |
| `(Record.pop r z)` | `(tuple (. r z) (Record.without r (z)))` — the **value** of `z` paired with the record of the **remaining** fields | `CDZ0212` if `z` is absent from `r` |

`extend` and `with` are deliberately **distinct**: `extend` requires the field to be **absent** (so an
accidental overwrite is a compile-time `CDZ0211`), `with` requires it to be **present** (so an
accidental introduction is a compile-time `CDZ0212`). This makes the author's intent — grow the shape,
or change a value — legible at the call and statically checked, rather than collapsed into one
add-or-replace form that silently does whichever the runtime shape happens to allow. `with` may change
the field's type because the result is a new closed record whose field `z` has whatever type `v` holds.

`Record.pop` needs **no `Option`**: whether `z` is present is a static property of `r`'s row, so a
missing field is a compile-time `CDZ0212`, not a runtime `None`. This is the row-typed counterpart of
`List.at`'s fallible `Option` return — a list index is runtime data, a record field name is not.

## Tuple primitives and derived operations

Tuples reshape **positionally**: a tuple's arity is part of its type, so every result arity is fixed
statically and there is no disjointness constraint (positions are anonymous).

| Form | Yields | Rejects |
|---|---|---|
| `(Tuple.cat t s)` | a tuple of `t`'s elements followed by `s`'s — arity `len(t) + len(s)`, each element keeping its source position's type | — |
| `(Tuple.split-at t k)` | `(tuple <prefix> <suffix>)` — the first `k` elements of `t` as one tuple, the rest as another | type error if `k` is not in `0..=len(t)` (the `tuple.N`-style static bounds rule) |
| `(Tuple.pop t)` | `(tuple (tuple.0 t) <rest>)` — element 0 paired with the tuple of the remaining elements | type error if `t` is empty (unit has no element 0) |

`k` is a **compile-time** position, written as a literal, exactly as `tuple.N` writes its index; a
split at `0` yields `(tuple () t)` (an empty prefix) and a split at `len(t)` yields `(tuple t ())` (an
empty suffix). `Tuple.pop t` is `(Tuple.split-at t 1)` with the singleton prefix unwrapped to its
element — the positional analogue of `Record.pop`.

## Why these operations, and why derived-not-primitive

- **`project` / `without` / `merge` are the row algebra.** Restriction, difference, and (disjoint)
  union over a labelled row are the minimal complete set: every other record reshape is a composition of
  them. Pinning three primitives and deriving the rest keeps the semantics small and the corpus honest —
  a behavior-gate failure names the primitive that broke.
- **`extend` / `with` / `pop` are the ergonomic surface.** They are what an author reaches for
  (add a field, update a field, take a field off), and each carries a *stricter* static contract than
  its rewrite makes obvious — `extend` forbids clobber, `with` forbids introduction — so naming them is
  worth it over spelling out the `merge`/`without` composition at every site.
- **No overloaded `=`, no implicit widening.** These operations are the *only* things that change a
  record's shape, upholding the rows learning: `=` stays full structural equality over identical closed
  shapes, and subset comparison is `(= (Record.project r (x)) (Record.project s (x)))` — an explicit
  projection then a same-shape compare (type-system.md §"Records Are Rows" 4th sentence; the corpus's
  subset-comparison case, `15-rows-and-open-sums.sexp`, is the plain-`.` special case of this).

## Interaction with row polymorphism

Because the operations are row operations, a definition over them is **row-polymorphic** and inference
assigns it a principal type. `(def (stamp r) (Record.extend r (v 0)))` is inferred over "any record `r`
that lacks `v`", i.e. `∀ρ. (ρ ∌ v) ⇒ {ρ} → {v: Int64 | ρ}`, and each call site monomorphizes it to a
concrete closed shape before the component boundary (type-system.md §"Records Are Rows" 3rd sentence;
§"A Generic Definition Is Monomorphized Before The Component Boundary"). The `CDZ0211`/`CDZ0212`
rejections are the ground cases of the row constraints the disjointness (`ρ ∌ z`) and presence
(`z ∈ r`) conditions impose — a lacks/contains constraint that a monomorphized instantiation violates
is that instantiation's `CDZ0211`/`CDZ0212`, exactly as a failed generic constraint is a compile-time
rejection (type-system.md §"A Generic Constraint Is A Compile-Time Predicate Over Type-Values").

## Realization and gating

These operations ride on the row-polymorphism layer, which the seed does not realize
(`options/realized-capability-set/seed-ignition-set.md`: "type-system beyond the static-typing floor").
Their corpus cases carry **`(needs rows)`** — the same tag the existing open-record cases carry
(`15-rows-and-open-sums.sexp`) — so the seed skips them until a generation realizes row inference;
`Record.*`/`Tuple.*` are unbound names to the seed, so tagging them `(needs rows)` (not the realized
`collections`) keeps the seed from running them and rejecting the unbound prelude name as a gate FAIL,
the same discipline the `Set` cases use with `(needs sets)`.

## Resolved forks

- **Namespaced `Record.*`/`Tuple.*` forms** (not bare `merge`/`project`/`without`). The operations are
  reached through the `Record` and `Tuple` prelude records, mirroring `Set.insert`/`List.at`, so no new
  bare reserved head words are minted — a program is free to bind `merge` or `project` as ordinary
  names. They remain **special forms** despite the prefix, because a field-name list and a position are
  static operands the compiler resolves, not runtime values a function receives.
- **Distinct `extend` and `with`** (not one add-or-replace `with`/spread). A single left-biased
  add-or-replace form (JS `{...r, z: v}`) was rejected because it silently overwrites — the exact
  no-silent-clobber discipline the strict `merge` exists to enforce. Splitting the intent into `extend`
  (grow, field must be absent → `CDZ0211` on clobber) and `with` (change, field must be present →
  `CDZ0212` on introduction) makes each call's intent legible and statically checked. A convenience
  add-or-replace MAY be offered later only as an elaboration that provably rewrites to these without
  changing emitted bytes.
- **Strict unbiased `merge`** (not left- or right-biased union). Because a record's fields are a set,
  `merge` of two records that share a field has no non-arbitrary value to keep, so it is a `CDZ0211`
  rejection rather than a silent pick — the row analogue of the duplicate-field literal `CDZ0201`.
- **Three record primitives + two tuple primitives, everything else derived** (not a flat list of
  independent builtins). Deriving `extend`/`with`/`pop` (and `Tuple.pop`) by a meaning-preserving
  rewrite keeps the primitive semantics minimal and the derived operations provably equal to their
  rewrite, the same explicit-then-elaboration posture the trait layer takes.
- **`pop` is row-typed, not `Option`-returning** (contrast `List.at`). A record field name is static,
  so field presence is a compile-time property and a missing field is `CDZ0212`, not a runtime `None` —
  `pop` needs no fallible return. Fallibility is for runtime indices (`List.at`, `Map.get`), not static
  labels.
