# Four parallel semantics drifted

*2026-07-02*

**What happened.** The meaning of the Cadenza language lived in four places at once: the
tree-walking interpreter's code, a separate `docs/semantics/` document with embedded input/output
examples, the meta-compiler's semantics-as-data definitions, and a K-framework formal model. Each was
a plausible authority for "what a construct does," and they drifted — a behavior fixed in the
interpreter was not reflected in the document, the meta-compiler implemented a subset, and the formal
model lagged far behind. There was no single answer to "what does this construct mean," only several
answers that disagreed.

**Why.** No one artifact was designated the single source of truth for behavior, and none was
*executable as the authority*. A document can drift from code because neither is required to agree
with the other; a formal model can lag because nothing fails when it does. Behavior defined in more
than one place, with no mechanism forcing agreement, always diverges.

**The requirement it drove.** [Core Principle IX](../../constitution.md) "Behavior Has One Executable
Semantics" and the [`spec/semantics/`](../semantics/) corpus as that single source of truth, gated by
execution rather than by extraction: every case must run to its recorded output
([conformance-gate.md](../capabilities/conformance-gate.md) §"The Behavior Gate"), and the compiler
and every tool agree with the corpus rather than encode their own behavior
([core-semantics.md](../capabilities/core-semantics.md) §"Evaluation Is Deferred To The Corpus";
[tooling-and-lsp.md](../capabilities/tooling-and-lsp.md) §"Tooling Shares The Compiler And The
Semantics"). The reference interpreter realizes the corpus and is the behavioral oracle.
