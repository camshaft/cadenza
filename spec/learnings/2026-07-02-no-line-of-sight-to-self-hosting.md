# There was no line of sight to self-hosting

*2026-07-02*

**What happened.** Across earlier Cadenza generations there was no concrete path from the language to
the language building itself. The compiler was a foreign-language program, the semantics lived in
documents and models that were not runnable as the authority, and nothing connected "here is the
language" to "here is the language authored in itself and improving itself." A system meant to power a
flywheel — agents proposing and improving behavior — had no seam through which Cadenza could become
the thing agents iterate on.

**Why.** Self-hosting was never made a first-class requirement, so nothing forced the design to admit
it. Without a designated behavioral oracle that could be authored in Cadenza, and without a staged
plan from a foreign seed to a Cadenza-authored compiler, self-hosting was an aspiration with no
mechanism — and a mechanism that is not required is not built.

**The requirement it drove.** [Core Principle XIV](../../constitution.md) "The Language Has A Line Of
Sight To Self-Hosting," realized by [self-hosting-and-bootstrap.md](../capabilities/self-hosting-and-bootstrap.md)
and [bootstrap.md](../bootstrap.md): the reference interpreter realizes the one executable semantics
and is the behavioral oracle a compiled program must agree with; a component may be derived by
embedding that interpreter over source, so a working component exists before ahead-of-time
compilation is complete; and each generation of the toolchain after the operator-synthesized seed is
derivable by the one before it, until the compiler is authored in Cadenza. The
[bootstrap-strategy/](../../options/bootstrap-strategy/) default pins the staged plan.
