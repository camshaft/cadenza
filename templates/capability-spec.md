# Capability — [NAME]

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines [one-line summary of what this capability does]. Requirements realize
> [Core Principle N](../../constitution.md) and trace to [overview §N](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

<!--
  AUTHORING CHECKLIST (delete before finalizing):
  - Every normative statement is ONE self-contained, ATOMIC RFC-2119 sentence under a stable
    heading. No compound requirements: split any sentence that joins two independent obligations.
  - Vocabulary comes only from spec/glossary.md; add new terms there first.
  - Standalone: no concrete engine, algorithm, numeric width, prior prototype, or file path. Name
    concrete realizations only at the declared-default location.
  - State WHAT must hold (behavior + invariants), never HOW (implementation).
  - Every behavioral requirement is witnessed by at least one case in spec/semantics/, so the
    behavior gate exercises what the requirement gate cites.
  - After writing: add to spec/traceability.md (both directions), add a [[specification]] block to
    .duvet/config.toml with format = "markdown", and confirm both gates cover it.
-->

## Purpose And Scope

[What this capability provides, where its boundary is, and which contracts or capabilities it relies
on. One or two short paragraphs.]

## [Section Name]

### [Stable Subsection Heading]

[The behavior] MUST [the single, self-contained requirement].

<!--
  OPTIONAL CAPABILITY (delete this block if the capability is always built).
  A capability a build may opt out of declares itself optional so the gate does not fail a build
  that excludes it (conformance-gate.md §"An Excluded Optional Capability Is Not Load-Bearing").

  ## Optionality

  ### This Capability Is Optional

  [This capability] MUST be an optional capability a build may include or exclude, in accordance
  with the build's declared defaults.

  ### The Declared Default Is [Include | Exclude]

  When a build is not told whether to include [this capability], it MUST [include | exclude] it.
-->
