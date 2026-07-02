# Decision — Code Shape

**The decision.** What the canonical representation of a Cadenza program *is*, and what relationship
holds between the form an agent manipulates and the form a human reads. The constitution requires a
canonical textual form that round-trips byte-for-byte and a structural interface for manipulating
programs (constitution X), but it does not fix a representation or a display, because those are
replaceable choices this decision pins.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- A canonical textual form that round-trips (constitution X: formatting yields byte-identical
  canonical bytes; parse-then-format reproduces them).
- A structural interface for reading and rewriting program structure without re-parsing unrelated
  code (constitution X; agent-authoring.md).
- Written and read by agents; read by humans — the top two north-star priorities.
- Reproducible codegen — the representation and its displays must not make canonical round-trip or
  structural edits fragile (reproducible-derivation.md).

**Why this is an isolated decision.** The whole specification is written against "the canonical
representation" and "the canonical textual form," so changing the representation or a display is an
edit to a choice file here plus a formatter and parser — it touches no frozen contract and no
capability requirement.

## Choices

- [`homoiconic-decoupled-display`](./homoiconic-decoupled-display.md) — a homoiconic canonical
  representation with display decoupled from it; any number of displays project from one
  representation. **The default.**
- [`conventional-primary`](./conventional-primary.md) — a conventional ML/Rust-family surface as the
  primary form over a typed tree, with a homoiconic literal as a secondary view.
- [`homoiconic-only`](./homoiconic-only.md) — s-expressions as the sole surface, no conventional
  display.

DEFAULT: homoiconic-decoupled-display
