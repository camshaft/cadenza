# Decision — Code Shape

**The decision.** What the canonical representation of a Cadenza program *is*, and what relationship
holds between the form an agent manipulates and the form a human reads. The constitution fixes that
the canonical stored form is a binary serialization of the AST and that a textual syntax is a lossless
projection of it (constitution X; ast-encoding.md), but it does not fix the shape of the
representation or which textual syntaxes are offered, because those are the choices this decision
pins.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- The canonical stored form is the binary AST, and a textual syntax parses to and prints from it
  losslessly (constitution X; ast-encoding.md).
- A structural interface for reading and rewriting program structure without re-parsing unrelated
  code (constitution X; agent-authoring.md).
- Written and read by agents; read by humans — the top two north-star priorities.
- Reproducible codegen — the representation and its syntaxes must not make the round-trip or
  structural edits fragile (reproducible-derivation.md).

**Why this is an isolated decision.** The whole specification is written against "the canonical
representation" and "the binary AST," so changing the representation's shape or the set of textual
syntaxes is an edit to a choice file here plus a parser and printer — it touches no frozen contract
and no capability requirement.

## Choices

- [`homoiconic-decoupled-display`](./homoiconic-decoupled-display.md) — a homoiconic canonical
  representation with display decoupled from it; any number of displays project from one
  representation. **The default.**
- [`conventional-primary`](./conventional-primary.md) — a conventional ML/Rust-family surface as the
  primary form over a typed tree, with a homoiconic literal as a secondary view.
- [`homoiconic-only`](./homoiconic-only.md) — s-expressions as the sole surface, no conventional
  display.

DEFAULT: homoiconic-decoupled-display
