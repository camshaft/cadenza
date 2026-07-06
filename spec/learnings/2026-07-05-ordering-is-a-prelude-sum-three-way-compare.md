# Ordering is a prelude sum — a total order is surfaced as three-way `compare`, not only `<`

*2026-07-05*

**What happened.** The language pins a standard three-way comparison result: **`Ordering`**, a closed
prelude sum `(Less | Equal | Greater)`, and a `compare : (T, T) → Ordering` operation over any type
that offers a total order. This is **not a new primitive type** — it is an ordinary closed sum in the
prelude, alongside `Option`, `Result`, and `Sign` — and `compare` is not new machinery, it is the
single primitive from which the boolean operators `<` `>` `<=` `>=` `=` are all definable.

**Why.** Core-semantics already carries §"Ordering Where Offered Is Total" (a type that offers an
ordering offers a *total* order; Bool offers one with `false < true` —
[[2026-07-05-bool-offers-a-total-order]]), but the only *surface* of that order was the four boolean
operators, each collapsing the comparison to a single bit. A `List.sort`, a `Map`/`Set` keyed on a
canonical element order ([[set-is-a-primitive-collection-not-a-map-of-unit]]), or a binary-search step
wants the *three-way* answer in one comparison — "less, equal, or greater" — not two boolean probes
that recompute the same comparison. Every ordered language surfaces this (Rust `Ordering`, Haskell
`Ordering`, C `strcmp`'s sign); Cadenza had the total-order *property* with no value to name its
result.

**Why a sum, not an integer.** The C convention returns a signed int whose *sign* is the answer — a
representation that invites the classic bugs (comparing the result to a literal, arithmetic on it,
assuming a particular magnitude). `Ordering` as a three-variant closed sum makes the result
**exhaustively matchable**: a `compare` consumer writes `((Less _) …) ((Equal _) …) ((Greater _) …)`
and the match-exhaustiveness rule (core-semantics.md §"Matching Is Exhaustive Or Rejected") forces all
three cases to be handled. It reuses the sum machinery the language already has — nullary variants,
uniform constructor patterns — with zero new mechanism, exactly the uniformity `Sign` (Neg | Zero |
Pos) already demonstrates. `Ordering` *is* `Sign` for comparison results.

**`compare` is the primitive; the operators are derived.** The relationship is one direction:
`compare` yields the full order, and `<` `>` `<=` `>=` `=` are each a pattern on its result
(`(< a b)` ≡ `(match (compare a b) ((Less _) true) (_ false))`). This keeps a single source of truth
for a type's order — a type defines *one* comparison, and all five operators plus `Ordering`-returning
`compare` follow from it, so they can never disagree (the total-order and determinism obligations bind
`compare`, and the operators inherit them). It also gives the `Set`/`Map` canonical element order a
name: the deterministic key-derived order those collections serialize in *is* the order `compare`
defines.

**Which types offer it.** The same set that offers `<` today — the ground clause the Bool learning
started filling in: `Int`/`UInt` (all widths), `BigInt`, `Rational`, `Bool` (`false < true`), `Char`
(scalar-value order — [[char-is-a-validated-unicode-scalar-the-boundary-already-promises]]), `String`
(lexicographic over scalars, already pinned), and structurally, tuples/lists lexicographically over
element order. `Float64` ordering stays deliberately unspecified for now (NaN has no total order under
IEEE compare; the corpus already declines float ordering) — `compare` is offered only where a *total*
order exists, which is the exact precondition §"Ordering Where Offered Is Total" already states.

**Contract impact — none new.** `Ordering` is a closed prelude sum, so its canonical byte form and its
boundary representation are *already* fixed by the existing sum-type rows in deterministic-value-form
and type-mapping (`variant { … }`); adding the prelude binding introduces **no new contract surface**
and **no version increment**. It needs **no new diagnostic**: `compare` on two different types, or on a
type with no total order, is the ordinary type error the four operators already produce (`CDZ0201`).

**Realization / gating.** `compare` and `Ordering` ride the same order the comparison operators do;
cases carry no special `(needs …)` beyond what the operand type already requires (e.g. `Char` cases
carry `(needs collections)`), and a generation that declines an operand's order declines `compare` on
it identically ([[2026-07-03-decline-do-not-miscompile]]).

**The requirements it drove.** [core-semantics.md](../capabilities/core-semantics.md) §"Equality And
Ordering" gains §"A Total Order Is Observed Through A Three-Way Comparison": a type that offers a total
order MUST offer a `compare` yielding `Ordering` (Less | Equal | Greater), and the boolean ordering
operators MUST agree with it (be definable from it), so a type has one order surfaced two ways that
cannot diverge. `Ordering` is added to the prelude sums (with `Option`/`Result`/`Sign`). Corpus
witness: `compare` cases in `03-equality-and-observation.sexp` — `(compare 1 2)` → `(Less unit)`,
`(compare 2 2)` → `(Equal unit)`, `(compare 3 2)` → `(Greater unit)`, agreement with `<`/`=`, and a
`compare` over `Char`/`String` — matched exhaustively to prove the three-variant dispatch.
