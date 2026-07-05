# `options/` — Decisions, Their Choices, And Declared Defaults

**What this directory is.** The concrete, technology-named resolutions of the decisions the
specification deliberately leaves open. The specification under [`spec/`](../spec/) and the
[constitution](../constitution.md) state **requirements** — technology-neutral, RFC-2119, and
gate-extractable. This directory states, for each open **decision**, the candidate **choices** that
satisfy those requirements with specific technology, and names the **default** choice. It is the
declared-default location the constitution's standalone rule points to when a requirement may not name
an implementation choice.

## Layout

```
options/
  <decision>/
    README.md          # the decision: the requirements a choice must satisfy, and DEFAULT: <choice>
    <choice-a>.md      # a full candidate realization of the decision
    <choice-b>.md      # an alternative realization, where genuine alternatives exist
```

Each `<decision>/README.md` states the requirements any choice must satisfy, lists the available
choices, and names the default with a line `DEFAULT: <choice>`. Each `<choice>.md` is a complete
realization — everything a build needs to adopt that choice.

## How a build selects a choice

- **Autonomous mode** applies each decision's `DEFAULT:` choice without asking, and records which
  choice it applied, so two unattended builds of the same specification resolve every decision
  identically.
- **Attended mode** MAY surface a decision's choices to the operator, who can pick a listed choice or
  **author a new `<choice>.md` for that one decision** and select it. Designing a fresh choice affects
  only its own decision directory; every other decision keeps its default.
- Deleting a `<decision>/` directory tells the build to treat that decision as unresolved and
  investigate it from first principles against the requirements; an autonomous build cannot proceed
  with a decision unresolved, because it would have no default to apply.

## Why decisions are separated from the spec

A good specification states requirements and does not overprescribe a technology. Keeping the concrete
choices here — not in the frozen-contract prose — lets the contracts stay standalone and
technology-neutral while still being **pinned** to specific artifacts, which cross-generation
interoperability requires. This directory is committed (unlike the gitignored `implementation/`),
because some of these pins are durable: a component derived under them must remain interoperable with
any future generation.

## The decisions

| Decision | Default choice | Realizes requirements in |
|---|---|---|
| [`ast-encoding`](./ast-encoding/) | binary-sexpr | ast-encoding.md; source-tree-encoding.md |
| [`execution-model`](./execution-model/) | wasm-component-model | host-interface-binding.md; determinism-and-fuel.md; build-tool-interface.md |
| [`code-shape`](./code-shape/) | homoiconic-decoupled-display | constitution X; core-semantics.md; agent-authoring.md |
| [`numeric-model`](./numeric-model/) | explicit-checked | numeric-model.md; deterministic-value-form.md |
| [`type-mapping`](./type-mapping/) | component-model-types | component-abi.md |
| [`hashing-and-encoding`](./hashing-and-encoding/) | sha256-deterministic-cbor | source-tree-encoding.md; deterministic-value-form.md; reproducible-derivation.md |
| [`diagnostics-schema`](./diagnostics-schema/) | coded-span-record | diagnostics.md |
| [`toolchain`](./toolchain/) | pinned-identity | reproducible-derivation.md; build-tool-interface.md |
| [`bootstrap-strategy`](./bootstrap-strategy/) | rust-seed-interpreted-first | bootstrap.md; self-hosting-and-bootstrap.md |
| [`structural-interface`](./structural-interface/) | content-addressed-nodes | agent-authoring.md |
| [`gate-non-load-bearing`](./gate-non-load-bearing/) | change-process-and-excluded | conformance-gate.md |
| [`realized-capability-set`](./realized-capability-set/) | seed-ignition-set | conformance-gate.md; self-hosting-and-bootstrap.md |
| [`self-hosting-surface`](./self-hosting-surface/) | minimal-reflective-surface | self-hosting-surface.md; self-hosting-and-bootstrap.md |
| [`effects-model`](./effects-model/) | algebraic-one-shot | capabilities-and-effects.md; host-interface-binding.md |
| [`memory-ownership-model`](./memory-ownership-model/) | reference-counting-perceus | memory-and-resource-model.md |
| [`ad-hoc-polymorphism`](./ad-hoc-polymorphism/) | explicit-dictionaries | type-system.md |
| [`verification-strategy`](./verification-strategy/) | liquid-refinements-extrinsic-proofs | verification-layers.md; property-based-testing.md |

## Change discipline

A change that alters bytes produced from unchanged source — the type mapping, the hashing or encoding
rule, the numeric byte forms — is an ABI-level change subject to the constitution's Governance Floor
"The Component ABI Changes Only By Coordinated Act." A change to a more-replaceable choice (the
execution engine, the seed language) must still satisfy every requirement it realizes but does not by
itself alter already-derived components.
