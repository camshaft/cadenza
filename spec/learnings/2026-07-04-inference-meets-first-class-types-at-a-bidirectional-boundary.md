# HM inference and first-class types meet at a bidirectional-checking boundary

*2026-07-04*

**What happened.** Two commitments already in the tree are in direct tension, and the resolution is
pinned: **HM principal-type inference by unification** ([[2026-07-04-inference-is-hindley-milner]]) lives
*alongside* **types as first-class values that can be computed** and **generics as type-valued
parameters** ([[2026-07-04-generics-are-type-valued-parameters]]). Full HM has principal types *because*
its type language is simple and non-computational; the moment types are values computed by arbitrary
compile-time code, principal types cease to exist and inference is undecidable in general (System F
already lacks full inference; anything dependent-flavored definitively so). Taken literally, the
requirement "type inference MUST determine the *principal* type by unification" is unimplementable the
first time a real generic appears.

**Why.** The gap is not a mistake in either commitment — each is right on its own — it is a missing
statement of *where they meet*. Every language that has both a Hindley-Milner core and richer type-level
expressiveness resolves it the same way, and Cadenza should say so explicitly: **HM inference over a
predicative, non-computational term-level core, with a bidirectional-checking boundary at positions that
take a type-valued parameter.**
- In the **HM fragment** — ordinary terms, monomorphic and let-generalized bindings, records as rows
  ([[2026-07-04-records-are-rows-open-by-default]]) — inference is full and principal, exactly as the
  Inference section already requires.
- At a **type-valued-parameter position** — where a generic's type argument or a computed type flows in
  — the type is either **synthesized** when monomorphization pins it from the surrounding uses, or
  **checked against an annotation** the author supplies. This is standard **bidirectional typing**: the
  system switches from "infer" to "check" precisely at the positions HM cannot principally infer, rather
  than pretending to infer them. Rust's turbofish, F#'s annotations where SRTP can't resolve, and
  Agda/Idris explicit arguments are the same boundary made visible.

**Consequences.**
- **The "principal type" requirement is scoped, not weakened.** Inference yields the principal type
  *within the HM fragment*; a type-valued-parameter position is resolved by checking or by
  monomorphization, not by principal inference. Without this scoping the requirement over-promises; with
  it, the requirement is exactly what a real implementation can deliver.
- **It keeps the dependent-ish danger quarantined.** First-class type *computation* stays behind the
  checking boundary, so it never forces the inference engine to be a decision procedure for type
  equality of arbitrary computed types — the engine stays HM-plus-rows, which is decidable and
  principal (Rémy).
- **It composes with erasure and monomorphization.** Because a type-valued parameter must resolve to a
  concrete type at compile time (`type-system.md` already requires this), the checking boundary always
  bottoms out in a monomorphic, erasable type before the component boundary
  ([[2026-07-04-static-typing-is-mandatory-post-pivot]]) — the bidirectional boundary and the
  monomorphization boundary are the same boundary.
- **It tells an implementer what to build.** A concrete compiler grows a unification-based inference
  pass over the term core and a check-mode entry point at annotation and type-argument sites — not a
  single omniscient inference pass over a computational type universe (which cannot exist). This is the
  clarification that most unblocks a tractable type-checker.

**Prior art.** Bidirectional typing (Pierce–Turner, Dunfield–Krishnaswami), **OCaml/SML** for the HM
core, **Rust** and **F#** for the "infer where you can, annotate at the type-argument boundary"
ergonomics, and the **predicative** discipline that keeps the core decidable.

**The requirements it drives.** `spec/capabilities/type-system.md` §Inference is annotated to scope
"principal type" to the HM fragment and to state the bidirectional boundary: a type-valued-parameter
position is resolved by checking against an annotation or by monomorphization, not by principal
inference, and the checking boundary coincides with the compile-time resolution generics already
require. No change to the ground commitments — this states the seam between them, closing an area the
spec left contradictory. Composes with [[2026-07-04-inference-is-hindley-milner]] (the HM fragment),
[[2026-07-04-generics-are-type-valued-parameters]] (the type-valued positions), and
[[2026-07-04-records-are-rows-open-by-default]] (the rows that keep the HM fragment principal).
