# Decision — Hashing And Encoding

**The decision.** The concrete hash function and canonical encodings that realize the source-tree
hashing rule (source-tree-encoding.md), the canonical value byte form (deterministic-value-form.md),
and the content addressing reproducible derivation depends on (reproducible-derivation.md).

**Requirements any choice must satisfy (from the spec — do not weaken):**
- A source tree has one canonical encoding and a source hash that is a function of it
  (source-tree-encoding.md).
- A value has one canonical byte form; equal values encode identically (deterministic-value-form.md).
- Derivation is content-addressed and reproducible (reproducible-derivation.md).

This is an ABI/wire-level decision: these choices fix bytes and hashes that identify source and
components, so a change is a coordinated change under the constitution's Governance Floors, with a
migration path.

## Choices

- [`sha256-deterministic-cbor`](./sha256-deterministic-cbor.md) — SHA-256, deterministic CBOR for
  the canonical value form, NFC normalization of string-value contents, and a sorted-path source-tree
  encoding over per-module binary ASTs, aligned with the host's content addressing. **The default.**

DEFAULT: sha256-deterministic-cbor
