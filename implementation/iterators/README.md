# The Cadenza iterators library

Iterators and ranges for Cadenza, written in Cadenza (ML surface). Extracted into its own package
(operator directive, 2026-07-18) so the compiler-ml package stays laser-focused on the integer spine
— the iterator code is a pure prelude-only consumer with no dependency on the compiler pipeline, so it
lives as a sibling package (like `cad/`, `agent-harness/`).

Run the suite with `cdz test implementation/iterators` (over this directory's `Project.cdz`).

## The modules

The package holds **two iterator encodings** plus the operator-directed redesign, side by side while
the redesign matures:

- **`adapter.cdz`** — the **operator-directed model** (re-charter 2026-07-18): an iterator is an
  *adapter record* `(state, step)` where `step : state -> Option (elem, state)`. A combinator
  (`map`/`filter`/`take-while`) is a plain function that **wraps** the upstream `step` into a new
  `step` calling it directly; a consumer (`fold`/`count`/`sum`/…) **drives** the step to exhaustion.
  This is *ad-hoc polymorphism* — a record carrying a closure that implements the protocol — **not a
  trait system and not an enum-dispatched step interpreter**. The design goal is **fusion**: a
  `map().filter().fold()` chain collapses into the consumer's one loop, "as if hand-written", with the
  combinator functions marked `const` for zero cost. See
  `implementation/design/DESIGN-monomorphic-fusing-iterators-rcdzc.md`.
  **Currently monomorphic-Int64** (the forcing proof that the fused model runs on today's compiler);
  the generic (∀-element) form is gated on the recursive-generic monomorphization tie, and the
  zero-cost *fusion* on the const-closure-through-recursion specialization (both tracked upstream).

- **`iter.cdz`** — the original **reified** iterator (monomorphic-Int64): `Iter` is a *sum of
  step-shapes* (`MapI`/`FilterI`/`TakeI`/…) and a central `next` interprets one step. Lazy without a
  thunk (a transformer builds a wrapping variant, forcing nothing until pulled). Rich surface:
  map/filter/take/drop/take-while/chain/zip-with/scan/flat-map/step-by/… + the range family
  (`range`/`range-inclusive`/`range-step`/`range-down`) + Set/Map → iterator bridges. This is the
  encoding the operator's re-charter moves *away from* (the enum dispatch); it stays until the adapter
  model subsumes it.

- **`giter.cdz`** — a **generic** (∀a) strict cons-cell pull-iterator (`Nil | Cons(a, GIter(a))`),
  the element-polymorphic companion. Unblocked by the recursive-generic producer tie (rcdzc Part C).
  Eager, not lazy (the cons-cell is fully materialized). Composes at ≥2 element types in one program.

- **`giter-flat.cdz`** — the nested generic `flatten` (`GIter(GIter(a)) -> GIter(a)`), split out
  because its nested instantiation is heavy (per-module monomorphizer pressure).

## Status (2026-07-18)

The iterator vertical was **re-chartered** to the monomorphic/generic/fusing adapter model. The
same-state monomorphic adapter surface is complete, algebraic-law-protected, and adversarially
hardened; the two endgame items — the **generic** adapter (compound-state combinators like
`take`/`drop`/`zip` + any-element) and true **zero-cost fusion** — are gated on two upstream compiler
capabilities (recursive-generic monomorphization tie; const-closure-through-recursion specialization),
both witnessed and progressing.

## House rules (inherited from the compiler-ml port)

This is a **stress test of the language**: where Cadenza can't express something cleanly, **report it**
(a crisp repro filed to the shared queue) rather than contorting around it. Friction found is a
deliverable. Tests live same-file with their code (a cross-file test can't yet construct a type whose
variant shadows a prelude name); exercise the combinators via same-file `@test`s (the module exports
only `Type.*` — a combinator taking the iterator type as a param is not boundary-representable, so it
can't be exported).
