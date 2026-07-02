# `defaults/` — Declared-Default Realizations

**What this directory is.** The concrete, technology-named realizations of the choices the
specification deliberately leaves open. The specification under [`spec/`](../spec/) and the
[constitution](../constitution.md) state **requirements** — technology-neutral, RFC-2119, and
gate-extractable. This directory states the **default answers** that satisfy those requirements
with specific, citable technology. It is the declared-default location the constitution's
standalone rule points to when a requirement may not name an implementation choice.

**Why it is separate from the spec.** A good specification states requirements and does not
overprescribe a technology. Keeping the concrete choices here — not in the frozen-contract prose —
lets the contracts stay standalone and technology-neutral while still being **pinned** to specific
artifacts, which cross-generation interoperability requires. This directory is committed (unlike
the gitignored `implementation/`), because some of these pins are durable: a component derived
under them must remain interoperable with any future generation.

## Contents

| File | Realizes requirements in | Pins |
|---|---|---|
| [`execution-model.md`](./execution-model.md) | host-interface-binding.md; determinism-and-fuel.md; build-tool-interface.md | component format, runtime engine, determinism configuration, resource measure, host-interface version and its operations |
| [`code-shape.md`](./code-shape.md) | constitution X; core-semantics.md; agent-authoring.md | the canonical-tree-plus-surface decision, the surface family, and the round-trip discipline |
| [`numeric-model.md`](./numeric-model.md) | numeric-model.md; deterministic-value-form.md | integer widths and overflow, floating-point mode, exact and rational representation, no implicit promotion |
| [`type-mapping.md`](./type-mapping.md) | component-abi.md | the concrete Cadenza-to-host-interface type table |
| [`hashing-and-encoding.md`](./hashing-and-encoding.md) | source-tree-encoding.md; deterministic-value-form.md; reproducible-derivation.md | the hash function, the canonical value encoding, and the source-tree hashing rule |
| [`diagnostics-schema.md`](./diagnostics-schema.md) | diagnostics.md | the machine-readable diagnostic record: code namespace, span shape, severity set, rule reference |
| [`toolchain.md`](./toolchain.md) | reproducible-derivation.md; build-tool-interface.md | the pinned compiler-toolchain identity for reproducing the compiler itself |
| [`bootstrap-strategy.md`](./bootstrap-strategy.md) | bootstrap.md; self-hosting-and-bootstrap.md | the seed language, the two derivation modes and their default, and the staged self-hosting plan |

**The guiding principles** (for every choice here): prefer determinism and reproducibility over
performance where they conflict; prefer explicitness over inference where a surprising inference
would cost verifiability; prefer cryptographic hashing and a single canonical binary encoding.

## How much control do you want over these?

1. **Accept the defaults (recommended).** Take everything here as-is; an autonomous build proceeds
   using these pins.
2. **Tune.** Change specific values (a different runtime engine, a different surface family) while
   keeping the rest. A change to a value that alters emitted bytes for unchanged source — the
   component ABI type table, the hashing rule, the canonical value form — is a frozen-root change
   subject to the constitution's Governance Floors and needs a stated migration path.
3. **Start from scratch.** Delete this directory; the build then treats every choice here as
   unresolved and investigates each from first principles against the spec's requirements. An
   autonomous build cannot proceed with `defaults/` deleted, because it would have no declared
   default to apply.

## Change discipline

A change that alters bytes produced from unchanged source — the type mapping in `type-mapping.md`,
the hashing or encoding rule in `hashing-and-encoding.md`, the numeric byte forms in
`numeric-model.md` — is an ABI-level change subject to the constitution's Governance Floor "The
Component ABI Changes Only By Coordinated Act." A change to a more-replaceable choice
(`execution-model.md` engine, `bootstrap-strategy.md` seed language) must still satisfy every
requirement it realizes but does not by itself alter already-derived components.
