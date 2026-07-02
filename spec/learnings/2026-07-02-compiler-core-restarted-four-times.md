# The compiler core was restarted four times

*2026-07-02*

**What happened.** Across earlier generations of Cadenza, the compiler core was rebuilt from scratch
at least four times: an imperative tree-walking interpreter (`cadenza-eval`), a Salsa-based
incremental core, a declarative "meta-compiler" that generated query implementations from
semantics-as-data, and a fresh Carp-style `Object` crate — with a K-framework formal semantics
alongside. Each restart re-derived the language's meaning and machinery, and each discarded the
intent accumulated in the one before it. The working artifact at any moment was whichever core was
current; the earlier cores became dead weight.

**Why.** The compiler was treated as the durable artifact. When the compiler is the thing you keep,
a better idea about how to build it means throwing away the compiler — and with it the accumulated,
undocumented decisions embedded in its code. There was no artifact more durable than the
implementation for those decisions to live in, so every architectural rethink was a rewrite that
started over rather than a regeneration from a specification that survived.

**The requirement it drove.** [Core Principle XII](../../constitution.md) "Specifications Are The
Durable Artifact": the compiler is a regenerable projection of the specification rather than the
source of truth, a defect is fixed in the spec and regenerated, and a generation is promoted only
when its load-bearing requirements are cited by an implementation and a test. The whole
specification-tree structure — a constitution, frozen contracts, capability specs, and one
executable semantics that outlive any compiler — is the response to this lesson.
