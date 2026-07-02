# Frozen Contract — [NAME]

> **FROZEN CONTRACT.** This document pins [what byte- or ABI-level form this contract fixes]. It is
> versioned and changed only by the coordinated act described in the constitution's Governance
> Floors. Its requirements realize [Core Principle N](../../constitution.md) and trace to
> [overview §N](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading.

<!--
  AUTHORING CHECKLIST (delete before finalizing):
  - A frozen contract pins something honored across every regeneration of the compiler: a byte
    form, the component ABI, a canonical encoding, a hash rule, or the tool interface.
  - Every normative statement is ONE self-contained RFC-2119 sentence under a stable heading.
  - Standalone: fix the SEMANTICS and the WIRE/ABI SHAPE, not a concrete engine, algorithm, or
    numeric width. Name the concrete realization only at the declared-default location.
  - Include an "Additive Evolution" section: a change is additive with respect to already-derived
    components, or carries a version increment and a migration path.
  - After writing: add to spec/traceability.md, add a [[specification]] block to .duvet/config.toml
    with format = "markdown", and if it is part of the seed subset add it to .duvet/bootstrap.toml.
-->

## Purpose And Scope

[What this contract fixes and why it is frozen. State explicitly what is pinned here versus what is
left to the capability specifications or the declared defaults.]

## [Section Name]

### [Stable Subsection Heading]

[The frozen shape or rule] MUST [the single, self-contained requirement].

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract MUST be additive with respect to already-derived components, or else carry an explicit version increment and a stated migration path.
