# The refinement layer is liquid types; machine-checked verification is extrinsic, by certificate

*2026-07-04*

**What happened.** The refinement-type and proof layers already named in `verification-layers.md` get
a direction: the refinement layer is realized as **liquid types** (refinement types over a *decidable*
predicate logic, discharged by SMT), and machine-checked verification is **extrinsic** — properties are
about a program's *values and behavior*, discharged into a **reproducibly-checkable certificate**, not
encoded as propositions-as-types in the type universe. This sets the course for the proof layer without
building it yet.

**Why liquid, specifically.** The choice lines up with commitments already in the tree:
- **It fits the certificate model the spec already wrote.** `verification-layers.md` already requires
  that "a statically discharged obligation MUST be recorded as a certificate whose validation does not
  depend on a nondeterministic solver run" and that "a verifier MUST be able to check a discharge
  certificate reproducibly." That is written *for* SMT-based verification: the solver searches
  (nondeterministically, off the byte path — the layer is meaning-preserving and does not change emitted
  bytes), but what is recorded and re-checked is a deterministic **certificate** (proof term / unsat
  core / checkable witness). Liquid types are the concrete realization of that already-stated intent.
- **Decidability matches the language's bounded/deterministic discipline.** The "liquid" restriction —
  predicates drawn from a decidable theory (quantifier-free linear arithmetic + uninterpreted
  functions) — is what makes checking terminate and be reproducible, the same shape as the
  fuel-bounded, halts-at-a-defined-point rules elsewhere. Full dependent refinements are undecidable;
  liquid is the decidable, *inferable* sweet spot (predicate abstraction infers refinements, serving the
  language's minimal-ceremony goal — [[2026-07-04-inference-is-hindley-milner]]).
- **It pays off twice, because refinements already drive property testing.** `property-based-testing.md`
  §"Refinements Constrain Generation" already requires a generator for a refined type to produce only
  values satisfying the refinement. A liquid predicate drives *both* the static SMT discharge *and* the
  dynamic generator — one predicate, two mechanisms.

**Why extrinsic, and how it defuses `Type : Type`.** `type-system.md` requires the type of a type-value
to be **the type of types** — i.e. `Type : Type`. That is a fine, pragmatic choice for a *programming*
language and it keeps first-class types cheap ([[2026-07-04-generics-are-type-valued-parameters]]) — but
it makes the type theory logically **inconsistent** (Girard's paradox), so a Curry-Howard proof layer
built *on the type system* (propositions-as-types) would be unsound: one could "prove" false.
Idris/Agda/Lean/Coq avoid this by stratifying universes (`Type₀ : Type₁ : …`) — i.e. by building a
dependent proof assistant. Cadenza takes the other fork: **proofs are extrinsic**, stated as contracts
and refinements *about program behavior* and discharged by a solver into a certificate (the shape
`verification-layers.md` already describes with its "certificate/witness" language). Because proofs live
*outside* the type system, `Type : Type` never endangers soundness. This is the natural fit for the
target domain — the references are **LiquidHaskell** (inference of refinements), **Flux** (liquid types
for Rust), and **Dafny** and the **Move Prover** (SMT-based extrinsic verification built for smart
contracts); **F\*** marks where the decidable line is.

**Consequences and the requirement to hold.** The genuinely hard part, which must be a requirement and
not glossed: **discharge must produce a witness a simple deterministic checker validates** — not "the
solver returned sat/unsat." SMT solvers are not reproducible across versions and do not always emit
checkable proofs; the spec already forbids a nondeterministic solver run from being the thing a verifier
re-executes, so the requirement is proof-*producing* discharge (a certificate a small checker validates
reproducibly), with the solver confined to the off-byte-path search. `Type : Type` must be scoped in
`type-system.md` to the **term-level programming language** so it is not read as a promise about a proof
logic — the note that keeps the extrinsic door open without walking through the intrinsic one.

**The requirements it drives.** `spec/capabilities/verification-layers.md` — §"The Refinement-Type
Layer" is annotated that refinements are drawn from a decidable predicate logic (liquid), and §"Static
Discharge Is A Reproducibly Checkable Certificate" is sharpened to require **proof-producing** discharge
(a checkable witness, solver off the byte path). `spec/capabilities/type-system.md` §"Types Are
First-Class Values Whose Type Is The Type Of Types" gains a scoping note: `Type : Type` is a term-level
convenience and machine-checked verification is extrinsic, so no propositions-as-types soundness claim
rides on the type universe. Recorded as a new decision **`options/verification-strategy/`** —
`liquid-refinements-extrinsic` as the default. Composes with the linearity split
([[2026-07-04-linearity-is-surgical-not-core]]): liquid types check *value* properties; the optional
usage layer checks *usage* properties.
