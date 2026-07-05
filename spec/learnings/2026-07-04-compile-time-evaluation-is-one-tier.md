# Macros, generics, monomorphization, and const-folding are one compile-time evaluation tier

*2026-07-04*

**What happened.** Four features that grew independently in the tree are recognized as **one
mechanism**: a single tier of pure, bounded, deterministic evaluation of Cadenza *at compile time*.
- **Const-folding / lambda inlining** — what `cdz-rustc` already does to specialize statically-known
  applications.
- **Generics** — "an ordinary definition taking type-valued parameters," specialized by "the
  compile-time reduction the compiler already performs" ([[2026-07-04-generics-are-type-valued-parameters]]).
- **Generic constraints** — "a compile-time predicate over type-values," checked "by the same
  compile-time evaluation" (`type-system.md`).
- **Macros** — "a compile-time transformation that receives and produces values of the canonical
  representation" (`metaprogramming.md`).

Named as one tier, a **macro is not a bolted-on transformer**: it is an ordinary Cadenza function that
runs in the compile-time phase, over `Ast` values (an ordinary sum type —
[[2026-07-03-types-first-class-in-dynamic-seed]]), producing `Ast` values (via quasiquote —
[[2026-07-03-quasiquote-for-programmatic-ast-construction]]). Generics, monomorphization, constant
evaluation, and constraint checking are all *instances* of running that tier.

**Why.** The spec already asserted every piece of this separately — `metaprogramming.md`
§"Compile-Time Evaluation Is Pure" and §"…Is Bounded," and the generics learning's "monomorphization is
not a distinct lowering path… no new pass." But writing them as four subsystems invites four
implementations that drift — the exact failure the language was built to avoid
([[2026-07-02-parallel-semantics-drifted]]). Unifying them:
- **Reuses the engine already built.** `cdz-rustc`'s const-reduction / `resolve_lambda` machinery is
  the tier; macros and generics point at it rather than adding passes.
- **Fits Cadenza's substrate better than its inspirations.** Zig's `comptime`, Scala 3's `inline`, and
  Lisp all approximate "run the language at compile time," but Cadenza *already* has the three
  properties that make it clean — homoiconicity, first-class types, and "the AST is an ordinary sum
  type" — so the tier is a naming exercise, not new machinery.
- **Gives the phase model one home.** "When does a macro run relative to type-checking?" and "when is a
  generic reduced?" become the *same* question about *one* tier, answered once
  ([[2026-07-04-macro-phases-and-the-reader-stays-fixed]]).

**The invariants the tier must carry (already stated, now consolidated).**
- **Pure.** No ambient IO, no clock, no randomness — sharpened by the effects model to *runs in the
  empty (pure) effect row* ([[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]), so
  reproducibility is a *consequence* of the effect typing, not a separate assertion. No capability may
  be reached at compile time.
- **Bounded.** Accountable against the deterministic resource measure so it halts at a defined point
  (Constitution V) — macro expansion, generic reduction, and constant evaluation all share this bound.
- **Deterministic / reproducible.** The tier is a pure function of its input AST, so expansion +
  reduction produce the same result on every conforming compiler (`metaprogramming.md` §"Expansion Is
  Reproducible").
- **Feeds, does not bypass, the core guarantees.** The tier's output is type-checked, capability-checked,
  and determinism-checked exactly as if written directly (`metaprogramming.md` §"Expansion Precedes And
  Feeds The Core Guarantees"); it cannot manufacture authority the manifest lacks.

**The requirements it drives.** `spec/capabilities/metaprogramming.md` gains a §"Compile-Time
Evaluation Is One Tier" (or the §Compile-Time Evaluation section is reframed) stating that macro
expansion, generic reduction/monomorphization, constraint checking, and constant folding are the same
pure, bounded, deterministic compile-time evaluation of Cadenza — one mechanism, not parallel
subsystems — and that this tier runs in the empty effect row. `spec/capabilities/type-system.md`
(generics/constraints) and `spec/capabilities/compiler-pipeline.md` are annotated to point at the one
tier rather than describe separate reductions. Composes with
[[2026-07-04-macros-are-typed-and-hygienic]] (what the tier produces) and
[[2026-07-04-macro-phases-and-the-reader-stays-fixed]] (when the tier runs).
