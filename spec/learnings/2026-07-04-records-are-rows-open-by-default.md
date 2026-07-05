# Records are rows: row polymorphism does triple duty and rescues principal-type inference

*2026-07-04*

**What happened.** The record surface gains **row polymorphism** (Rémy/Wand-style row types): a record
type is a set of labelled fields *plus an optional row variable* standing for "the rest of the fields."
A function can be typed over "a record with at least `x` and `y` and any other fields ρ." The same row
machinery is adopted for **effect rows** ([[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]),
so one mechanism serves three purposes:

1. **Open records** — polymorphism over "a record that *has at least* these fields," not only exact
   shapes.
2. **Effect rows** — an effectful arrow `a -> b / {E…}` types its effects as a row of labels, so effect
   inference *is* row inference.
3. **Principal-type inference is preserved.** Row polymorphism is one of the few HM extensions that
   keeps inference decidable and principal (Rémy). This is the tractable resolution of the tension
   flagged in [[2026-07-04-inference-is-hindley-milner]]: unrestricted first-class-type *computation*
   endangers principal types, but *rows* do not — they extend HM without adding to that danger.

**Why.** Two forces drove it:
- **Agent-authored code wants "a record with these fields plus whatever."** The language is authored by
  agents ([[2026-07-03-one-accessor-modules-are-records]]); a closed-records-only world (the first cut:
  "a record has a fixed set of named fields," comparable "only when their field-name sets are identical")
  makes every helper over "an object that has an `id`" require the exact shape. Rows let a function
  accept any record that *contains* the fields it uses. Because **modules are records**, this also gives
  principled "a module exposing at least these exports" typing for free.
- **Subset comparison was requested and must not corrupt `=`.** The desire to compare records "in a
  subset way" must **not** be met by overloading `=` to sometimes ignore fields — that would break
  "equality agrees with the canonical byte form" (`core-semantics.md` §Equality Is Structural). Instead
  it is an **explicit projection then compare**: `(= (project r {x y}) (project s {x y}))`, where
  `project` narrows an open record to a closed sub-row. This mirrors the existing discipline that a
  program *explicitly asks* to cross a type boundary rather than doing so silently — the nominal
  tag-strip escape hatch ([[2026-07-04-nominal-is-orthogonal-tag-over-structural-types]]). `=` stays
  full structural equality over identical shapes; `project` is the only thing that changes the shape,
  and it is well-typed and inferable *because* of rows.

**Consequences and boundaries.**
- **Row variables never cross the component boundary.** Like generics, an open record is resolved to a
  concrete *closed* shape before emission — monomorphized by the same compile-time reduction
  ([[2026-07-04-generics-are-type-valued-parameters]]) — so the ABI still sees only closed
  `record { … }` shapes with canonically-ordered fields (`options/type-mapping/`). Rows are a
  compile-time typing device, erased like every other type ([[2026-07-04-static-typing-is-mandatory-post-pivot]]).
- **Records stay closed at the value level.** A *value* is still a record with a fixed field set; row
  polymorphism is about the *types* a function accepts, not about values that grow fields at runtime.
  So the runtime representation and structural equality are unchanged — this is additive over the
  existing record semantics, not a redefinition of records.
- **Exact-shape comparison is unchanged.** Two records are still comparable only when their field-name
  sets are identical (`type-system.md` §Structural Values Are Comparable Only When Their Shapes Match).
  Row polymorphism changes what *functions* accept, not when two concrete records may be compared.
- **This is the record analogue of the sum/nominal universe.** Records get row polymorphism (open over
  fields); sums may later get the dual (open over variants / polymorphic variants, OCaml-style) if a
  need appears — recorded as future work, not committed here.

**Prior art.** **PureScript** (row types for records and effects), **OCaml** (object rows; polymorphic
variants as the sum dual), and the **Rémy/Wand** row-type line. The effect-row reuse follows **Koka**.

**The requirements it drives.** `spec/capabilities/type-system.md` §"The Declarable Type Universe"
gains a **row-polymorphism** subsection: a record type MAY carry a row variable; a function MAY be typed
over records containing at least its used fields; row variables resolve to closed shapes before the
boundary; and an explicit `project`/narrowing operation produces a sub-row, comparison over which is
ordinary structural equality (so subset comparison is projection-then-`=`, never an overloaded `=`).
`core-semantics.md` §"Records, Maps, And Member Access" is annotated that fixed-field values are
unchanged and openness is a type-level property. Composes with the ad-hoc-polymorphism decision
([[2026-07-04-traits-are-dictionaries-scoped-not-coherent]]) and the effects model, which share the
row substrate.
