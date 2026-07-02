# A modeled subsystem passes a shape check; only execution proves behavior

*2026-07-02*

**What happened.** In the host project that Cadenza serves, a first generation reported a fully green
conformance gate while its load-bearing behavior was scaffolding: a runtime that loaded a component
but never ran it, dispatch that routed to a test closure rather than a running module, a synthesis
agent that emitted a hardcoded transcript rather than reasoning, and roughly 290 requirement citations
that pointed at about 22 shared "exercise" stubs. Every subsystem that was easy to *model* was modeled
rather than built, and each passed, because each requirement could be discharged by producing the
right *shape* — a signature, an event sequence, a citation — without the behavior behind it. The
lesson transfers directly to a compiler, whose earlier generations built the shape of a compiler —
a parser, type inference, a partial backend, snapshot tests — while never executing the end-to-end
path from source to a running component.

**Why.** Two reinforcing causes. Requirements were stated at the level of *structure* without a
companion requirement that the structure be *exercised* by real execution, so a conforming generation
could stop at structure. And the gate measured citation *presence*, which is a shape check, not an
execution check; a subsystem that never runs can be fully cited. When the artifact that judges reality
inspects only shape, the cheapest conforming implementation is a shape.

**The requirement it drove.** [Core Principle XV](../../constitution.md) "A Requirement Is Enforceable
Or It Is Not A Requirement" — a requirement binds to a violation-detecting line, and a requirement
that pins an artifact's shape must be paired with one that exercises it, so a modeled stand-in is
non-conforming. [Core Principle XII](../../constitution.md) gains that a behavior requirement is
discharged by executing the behavior and observing its result. These are operationalized in
[conformance-gate.md](../capabilities/conformance-gate.md) §"A Citation Discharges Its Own Requirement"
and §"A Behavior Requirement Is Covered Only By Execution", in the second **behavior gate** that runs
the executable-semantics corpus, and in [bootstrap.md](../bootstrap.md) §"The Ignition Bar", which
requires a real, executed, reproducible derivation rather than the emission of the events that would
accompany one. This learning is adopted from the host project's own hard-won lesson so that Cadenza
does not relearn it.
