# Never is the empty sum — the dual of Unit, surfaced from what the seed already has

*2026-07-05*

**What happened.** The type universe pins its bottom type: **`Never`**, the sum type with **zero
variants**. It is the exact dual of `Unit`: where `Unit` is the product with zero *fields* (the empty
tuple, `()` — core-semantics.md, and `05-compound-types.sexp` §"unit is the empty tuple"), `Never` is
the sum with zero *variants*. This is not new machinery — the seed's `codegen.rs` already carries a
`Kind::Never` for a divergent expression (one that always traps: `unreachable`, stack-polymorphic,
unifies with any expected type). This learning **surfaces that internal as a first-class type in the
prelude**, giving it a name, a canonical treatment, and corpus witness, rather than leaving it an
unnamed compiler-internal.

**Why the type universe already implies it.** type-system.md §"The Structural Types Are Record, Tuple,
And Sum" says a structural sum is "a sum of named variants." Nothing in that requirement demands *at
least one* variant — a sum over the empty variant set is a well-formed structural type, the natural
zero of the sum constructor exactly as the empty tuple is the natural zero of the product constructor.
So `Never` is not an addition to the type *grammar*; it is the already-admitted degenerate case, named.
The 3×2 nominal/structural grid ([[nominal-is-orthogonal-tag-over-structural-types]]) had its empty
*product* filled (Unit) and its empty *sum* unnamed; this fills the second cell.

**What having it buys.**
- **An honest type for divergence.** `(trap …)`, and `expect` applied to a `None`, produce no value —
  they diverge. Their type is `Never`, which — because it has no values — **unifies with any expected
  type** (there is no value to be wrong, so a diverging expression validates in any position). This is
  precisely the "unifies with any expected kind" behavior the seed's `Kind::Never` comment already
  describes; naming the type makes the principle a *specified* property (the type-theoretic bottom is
  the identity of unification join) rather than a codegen convenience.
- **Provably-impossible arms and total signatures written honestly.** A function that never returns
  normally (an event loop, a re-raising error path) has result type `Never`; a match arm the type
  system proves unreachable is one binding a `Never`-typed scrutinee. The author writes the type the
  program actually has instead of a fictional `Int64` that never materializes.
- **The empty sum is uninhabited, so `Never → T` is free.** There is a canonical absorbing map from
  `Never` to any type (match on zero variants — vacuously exhaustive, the dual of the canonical map
  `T → Unit` that discards). A match on a `Never` value needs **zero arms** and is *exhaustive by
  construction* (there are no variants to cover), the degenerate base case of core-semantics.md
  §"Matching Is Exhaustive Or Rejected" — a clarifying corner of the exhaustiveness rule, not an
  exception to it.

**What it is NOT.** `Never` is a **type**, not a value — it has no constructor and no literal, because
it has no inhabitants (that is the whole content of "empty sum"). This is the sharp line from `Unit`:
`Unit` has exactly one value (`unit` / `()`), so it is *constructible*; `Never` has zero values, so it
is *only ever a type* — the type of an expression that does not produce a value at all. A program can
never build a `Never`, only diverge at a position typed `Never`.

**Contract impact — none.** `Never` never crosses a component boundary as a value (there is no value
to cross), so it needs **no type-mapping row** and **no canonical byte form**: it is uninhabited, so
deterministic-value-form's "each serializable value has one canonical form" is vacuously satisfied —
there is nothing to serialize. It touches **no frozen contract** and needs **no new diagnostic**: a
program that (impossibly) tries to use a `Never` value is caught by ordinary type checking, and the
existing exhaustiveness code `CDZ0210` already governs matches (a zero-arm match on a `Never` is the
*passing* degenerate case, not a rejection).

**Realization / gating.** The seed *already realizes* the mechanism as `Kind::Never` — a whole-body
`Never` function is emitted with an arbitrary result type and just traps at runtime — so the
divergence behavior is not a later-generation concern; only the *surface name* `Never` and the
zero-arm-match-is-exhaustive rule are what a later generation binds in the prelude. Core cases (a trap
expression typed `Never` unifying into an `Int64` position) need no `(needs …)`; the zero-arm match on
an uninhabited scrutinee is a `(needs …)`-gated later-generation nicety.

**The requirements it drove.** [type-system.md](../capabilities/type-system.md) §"The Declarable Type
Universe" gains §"Never Is The Empty Sum": `Never` is the sum type with zero variants, uninhabited,
the type of a diverging expression; it unifies with any expected type; a match on a `Never`-typed
scrutinee is exhaustive with zero arms. It is noted as the dual of Unit under the structural-types
requirement. Corpus witness: cases in `07-type-system.sexp` — a `(trap …)`/`Never` expression used in
an `Int64` position type-checks (the divergent-unifies-with-anything property), placed beside the
existing never-crash `(+ 5)`/`(if …)` type cases the seed already exercises.
