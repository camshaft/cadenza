# Proposal: an iterator is a lazy pull-computation; every sequence produces one and every consumer takes one

*Draft for sign-off — NOT yet normative. 2026-07-14. Composes with `range-as-a-first-class-value.md`
(a `range` becomes the archetypal iterator producer). A runnable realization exists under
`implementation/compiler-ml/src/iter.cdz` (monomorphic spike) with the language gaps it hit recorded as
repros; see "What a stress-test implementation proved" below. When accepted, its requirements move into
`collections-and-text.md` (a new `## Iteration` section) and `type-system.md`'s declarable universe under
stable headings, and this file is retired to a learning.*

## The problem this resolves

The collections each expose their own *structural* operations — `List.push`/`at`/`len`,
`Map.insert`/`lookup`/`size`, `Set.contains`/`union`, `String.scalar-at`, `Bytes.at` — but there is **no
way to walk a sequence** and no unified abstraction over "a thing that yields elements in order." Every
higher-order traversal a real program needs — map, filter, fold, find, count — is absent, and the one
mention of `List.map` in the corpus is aspirational prose in a comment, not a realized op.

The naive fix is to add `map`/`filter`/`fold`/`each`/`find`/… as methods on *each* collection. That is an
N×M explosion (every traversal on every collection), it forces an intermediate collection at every step
(`xs |> map f |> filter p` allocates twice), it cannot express an infinite or generated sequence, and it
gives `List`, `Map`, `Set`, `String`, `Bytes`, and a `range` five unrelated traversal vocabularies for
what is one idea. The deeper fix is to name the idea once: **a lazy sequence you pull elements from.**

Two existing normative requirements are already *implicitly* about this and currently have no operation to
attach to: *Map Iteration Is Deterministic* and *Set Iteration Is Deterministic* both fix the order in
which entries "are visited" — but nothing in the language visits them. This proposal is the operation
those requirements were written for.

## The design

### An `Iter a` is a lazy computation, not a data value

An `Iter a` denotes a **lazy, ordered, pull-driven sequence** of elements of type `a`. It is a
first-class value: it can be bound, passed, and returned. But it is a *computation*, not *data* — it sits
with closures, not with lists:

- It is **opaque**: its representation is unspecified and unobservable, exactly as a list's is
  (*A List's Representation Is Unspecified And Unobservable*), so the compiler is free to fuse a
  transformer chain into one stepper with no intermediate collection.
- It has **no structural equality, no ordering, and no canonical byte form.** Two iterators that would
  yield the same elements are not thereby equal; comparing them would have to force them. Equality,
  rendering, and interchange are defined on *data*; an `Iter` is a suspended computation, so `=` on an
  `Iter` is a type error and matching on one is the same clean rejection as matching on a function
  (CDZ0203). To compare, render, or persist a sequence, `collect` it back into a `List`/`Set`/`Map`.

This is the load-bearing line: **data is produced from, transformed as, and consumed back out of a
computation.** A collection is data (comparable, renderable, canonical); an `Iter` is the computation that
walks it.

### The whole observable interface is one total step that returns the next state

An iterator is observed by exactly one operation:

```
next : ∀a. (Iter a) → Option (a, Iter a)
```

`(next it)` yields `None` when the sequence is exhausted, and `Some (elem, rest)` otherwise — the next
element paired with **a new iterator for the remainder.** The second component is the *continuation* (the
iterator's next state), **not** a re-wrapped element: pulling advances the sequence by returning where to
resume. This mirrors the collections exactly: it is *total* (an exhausted iterator is `None`, never a
trap — *Indexing And Lookup Are Fallible*), it is *immutable* (stepping produces a new `rest`, leaving
`it` unchanged, exactly as `List.push` yields a new list), and it reuses `Option` and the 2-tuple rather
than inventing a `Step` sum.

Because stepping is pure and returns the rest as a value, **an `Iter` is re-steppable and shareable**:
stepping `it` twice both observe the same first element, with no "iterator already consumed" hazard. This
falls directly out of immutability and is the property an imperative iterator cannot offer.

### Every sequence produces an iterator; the deterministic-order requirements bind here

Each collection gains **one producer**, `iter`, and nothing else changes on the collection:

```
(List.iter   xs)   → Iter a            ; elements in list order
(Set.iter    s)    → Iter a            ; elements in the deterministic order Set Iteration Is Deterministic fixes
(Map.iter    m)    → Iter (k, v)       ; entries as (key, value) tuples, in the order Map Iteration Is Deterministic fixes
(String.iter str)  → Iter Char         ; scalar values in order
(Bytes.iter  b)    → Iter Int64        ; bytes in order
(Range.iter  r)    → Iter Int64        ; positions [start, end), or empty — the archetypal producer
```

`Map.iter` and `Set.iter` iterating in their already-specified canonical order is not new behavior — it
is the operation the two existing determinism requirements were written to have. A `range` producing an
iterator is why the two proposals compose: a `range` is a *finite description* of a sequence, an `Iter` is
the *walk* of one. Generated sources (`Iter.empty`/`once`/`repeat`/`iterate`/`unfold`) round out the
producers; `unfold : s → (s → Option (a, s)) → Iter a` is the primitive the rest are sugar for — the dual
of `next`, and the direct expression of the "step returns the next state" shape above.

### Transformers are lazy `Iter → Iter`; consumers fold back to data or effects

Traversal lives on `Iter`, once, not on each collection. **Transformers** are lazy (they compute nothing
until pulled) and total: `map`, `filter`, `filter-map`, `flat-map`, `take`, `drop`, `take-while`, `zip`,
`enumerate`, `chain`. **Consumers** drive the pull and return data or a value: `fold` (the primitive),
`each`, `collect-list`/`collect-set`/`collect-map`, `count`, `find`, `any`/`all`. The pipe reads the way
it was built to: `(|> xs (List.iter) (Iter.map f) (Iter.filter p) (Iter.collect-list))`.

### Effects flow through iteration for free

Because `next` and every callback are ordinary functions, an iterator whose step or transformer callback
performs an effect **contributes that effect's row to whoever drives it** — the existing effect-row
inference handles streaming I/O with no new mechanism. A pure iterator has the empty row; an iterator that
reads from a host source carries that capability, and its consumer must account for it. The pull model
fixes the order: an effectful element's effects occur when it is pulled, front-to-back, so a run stays a
deterministic function of its ordered host responses. There is no separate "async iterator" or
"generator" concept — an effectful stepper *is* the generator, and the effect system already types it.

## Why this is the right shape

- **One abstraction, no N×M.** Traversal lives on `Iter`; a collection contributes only `iter` (and is a
  `collect` target). Adding a collection costs one producer; adding a traversal costs one `Iter` op.
- **Lazy and fusible for free**, via the opaque-representation principle lists already carry.
- **Pure, deterministic, re-steppable** — the "consumed iterator" bug cannot occur.
- **The value/computation line stays crisp** — `Iter` joins closures as non-comparable; `collect` is the
  one bridge back to data.
- **It realizes requirements already on the books** (the two "iteration is deterministic" rules) and
  unifies the range proposal with this one.

## What a stress-test implementation proved (2026-07-14)

A runnable realization was written in Cadenza itself (`implementation/compiler-ml/src/iter.cdz`) as a
stress test. Findings that shape the design and the migration:

- **The ideal thunk encoding is currently blocked.** `Iter a = Susp(Unit -> Option (a, Iter a))` is the
  most direct lazy form, but a `Unit`-parameter closure boxed into a heap sum declines ("a closure's
  parameter type has no machine representation" — `Ty::Unit` has no machine valtype at the closure
  boundary). Repro: `implementation/compiler-ml/repros/decline-unit-param-closure-boxed-in-sum.cdz`. The
  proposal does not depend on the thunk encoding — a **reified (defunctionalized)** encoding (an `Iter` is
  a sum of the known step-shapes, `next` interprets one) is equivalent, still lazy, and stores each
  closure over the *element* type rather than `Unit`. The reified iterator compiles and runs today
  (map/filter/take/drop/take-while/fold/count/collect/find/any/all; laziness verified by `take 3` of a
  million-element range pulling exactly 3). **Fixing the `Unit`-boundary gap would additionally enable the
  thunk encoding**, which is what gives heterogeneous `map : Iter a → (a → b) → Iter b`, `zip`, and
  `enumerate` (the reified encoding names the source type in its variant, so its `map` is homogeneous).
- **A fully generic `Iter a` is blocked by an inference gap.** A polymorphic iterator with composed
  transformers (`collect(map(filter(…)))`) leaves the element type argument undetermined through the
  `next`-recursion, even though the element type is pinned by the source. Repro:
  `implementation/compiler-ml/repros/decline-generic-iterator-composed-transformers.cdz`. The stress-test
  library is therefore monomorphic over `Int64` as a spike; **a generic iterator — the actual goal — is
  gated on closing that inference gap.**
- **Signature surface:** a lowercase type var in an annotation on a user generic type is rejected
  (`repros/reject-user-generic-type-var-in-annotation.cdz`); generic signatures must currently be left
  unannotated. Minor, but it means the documenting `next : Iter a → …` signature can't yet be written in
  source.

These are the concrete language-work items this proposal creates; none changes the design above.

## Migration plan (sequenced)

1. **Unblock the primitives (seed):** give `Unit` a zero-information machine slot at the boxed-closure
   boundary (enables the thunk encoding + heterogeneous `map`/`zip`); close the generic-argument inference
   gap for composed transformers (enables a generic `Iter a`). Both have minimal repros.
2. **Spec (normative):** add `## Iteration` to `collections-and-text.md` — `### An Iterator Is A Lazy
   Pull-Driven Sequence` (opaque-computation, one-total-step-returning-the-rest, not-data), `### Every
   Sequence Yields An Iterator In Its Defined Order` (wiring producers to the existing deterministic-order
   rules), `### Iteration Carries The Row Of Its Steps`; add `Iter` to `type-system.md`'s declarable
   universe as an opaque computation type with no equality/ordering/canonical form.
3. **Library:** the `Iter` module (`next`/`unfold` primitives + producers + transformers + consumers) and
   a `.iter` producer on each collection; `Iter` gets no `=`/render/encode arm (rejects like a function).
   Promote `implementation/compiler-ml/src/iter.cdz` from the monomorphic spike to the generic library
   once step 1 lands.
4. **Corpus:** producers over each collection; the deterministic Map/Set order observed *through* `iter`;
   laziness (an effectful `take` firing exactly N steps); re-stepping one `Iter` twice; the `=`/match-on-
   `Iter` rejection.

## Open questions for sign-off

- **`next`'s result shape: `Option (a, Iter a)` or a dedicated `Step`?** Recommendation: **`Option (a,
  Iter a)`** — reuses `Option` + tuple, matches the fallible-access idiom, and the tuple's second slot is
  the continuation (the state to resume from), which reads correctly.
- **`Map.iter` element: 2-tuple `(k, v)` or an `Entry` record?** Recommendation: **the tuple**, consistent
  with `Map.take`/`swap`.
- **Do collections keep any eager traversal?** Recommendation: **`Iter` is the only traversal vocabulary**
  — collections keep structural ops + `iter` + being a `collect` target; no `List.map`.
- **A `yield`-based generator surface later?** An effectful stepper already covers a generator that does
  I/O; a first-class `yield` that suspends a producer for a consumer is a possible future once the effect
  system reifies continuations (`Ty::Cont`) — out of scope here, and this design does not block it.
- **Termination.** An infinite iterator consumed eagerly does not terminate; that is host-owned fuel
  policy (Principle V retired), not a language-level error — worth a one-line spec note, not a rejection.
