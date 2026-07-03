# Traceability

> **What this document is.** The bidirectional map between the normative specifications and the
> architecture they realize. Every normative section traces to a section of
> [overview.md](./overview.md) (the intent arbiter), and every overview section is served by at least
> one normative section. This document is descriptive; it carries no requirements. It exists so that
> a change to intent and a change to a requirement can each be checked against the other.

## Normative document → overview sections it realizes

| Normative document | Realizes overview sections |
|---|---|
| `constitution.md` | §1, §2, §4, §5, §6, §7, §10, §11, §12, §13, §14, §15 |
| `spec/contracts/ast-encoding.md` | §3 |
| `spec/contracts/source-tree-encoding.md` | §3, §7 |
| `spec/contracts/component-abi.md` | §5, §6, §8 |
| `spec/contracts/deterministic-value-form.md` | §4 |
| `spec/contracts/host-interface-binding.md` | §6, §8 |
| `spec/contracts/determinism-and-fuel.md` | §4 |
| `spec/contracts/reproducible-derivation.md` | §7 |
| `spec/contracts/build-tool-interface.md` | §7, §9 |
| `spec/capabilities/core-semantics.md` | §3, §4, §10, §11 |
| `spec/capabilities/type-system.md` | §5 |
| `spec/capabilities/capabilities-and-effects.md` | §6, §12 |
| `spec/capabilities/memory-and-resource-model.md` | §4 |
| `spec/capabilities/numeric-model.md` | §4, §5 |
| `spec/capabilities/collections-and-text.md` | §4, §5 |
| `spec/capabilities/modules-and-namespaces.md` | §2, §7 |
| `spec/capabilities/metaprogramming.md` | §3, §13 |
| `spec/capabilities/self-hosting-and-bootstrap.md` | §10, §11, §15 |
| `spec/capabilities/bootstrap-interpreter.md` | §10, §11, §15 |
| `spec/capabilities/verification-layers.md` | §12 |
| `spec/capabilities/property-based-testing.md` | §12 |
| `spec/capabilities/diagnostics.md` | §13 |
| `spec/capabilities/compiler-pipeline.md` | §7, §10, §14 |
| `spec/capabilities/conformance-gate.md` | §14 |
| `spec/capabilities/build-modes.md` | §15 |
| `spec/capabilities/tooling-and-lsp.md` | §10, §13 |
| `spec/capabilities/agent-authoring.md` | §13 |
| `spec/capabilities/units-of-measure.md` | §5, §12 |
| `spec/bootstrap.md` | §11, §15 |

## Overview section → normative documents that serve it

| Overview section | Served by |
|---|---|
| §1 The one idea | constitution.md |
| §2 Why Cadenza exists | constitution.md; modules-and-namespaces.md |
| §3 Source, programs, and the canonical form | ast-encoding.md; source-tree-encoding.md; core-semantics.md; metaprogramming.md; agent-authoring.md |
| §4 Determinism and bounded execution | constitution.md; deterministic-value-form.md; determinism-and-fuel.md; memory-and-resource-model.md; numeric-model.md; collections-and-text.md; core-semantics.md |
| §5 Types | constitution.md; component-abi.md; type-system.md; numeric-model.md; collections-and-text.md; units-of-measure.md |
| §6 Capabilities and no ambient authority | constitution.md; component-abi.md; host-interface-binding.md; capabilities-and-effects.md |
| §7 Derivation and reproducibility | source-tree-encoding.md; reproducible-derivation.md; build-tool-interface.md; modules-and-namespaces.md; compiler-pipeline.md |
| §8 The component boundary | component-abi.md; host-interface-binding.md |
| §9 Cadenza as a replaceable build tool | build-tool-interface.md |
| §10 One executable semantics | constitution.md; core-semantics.md; self-hosting-and-bootstrap.md; bootstrap-interpreter.md; compiler-pipeline.md; tooling-and-lsp.md |
| §11 The reference interpreter as oracle | constitution.md; core-semantics.md; self-hosting-and-bootstrap.md; bootstrap-interpreter.md; bootstrap.md |
| §12 Progressive verification | constitution.md; capabilities-and-effects.md; verification-layers.md; property-based-testing.md; units-of-measure.md |
| §13 Authored for agents | constitution.md; metaprogramming.md; diagnostics.md; tooling-and-lsp.md; agent-authoring.md |
| §14 Conformance by two gates | constitution.md; compiler-pipeline.md; conformance-gate.md |
| §15 Self-regeneration: the flywheel | constitution.md; self-hosting-and-bootstrap.md; bootstrap-interpreter.md; bootstrap.md; build-modes.md |
| §16 What earlier generations taught | (descriptive; served by `spec/learnings/`) |
