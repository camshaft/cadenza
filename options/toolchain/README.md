# Decision — Toolchain

**The decision.** The pinned toolchain identity that realizes reproducible-derivation.md's
requirement that the same canonical source with the same pinned toolchain produces byte-identical
output, and that the compiler records the toolchain identity alongside each component it produces.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- Derivation is a function of source and toolchain; the toolchain identity is recorded alongside the
  component (reproducible-derivation.md).
- Anyone may re-derive and verify (reproducible-derivation.md).

## Choices

- [`pinned-identity`](./pinned-identity.md) — records the compiler component hash and host-language
  toolchain identity at both reproducibility levels, and normalizes provenance. **The default.**

DEFAULT: pinned-identity
