# Decision — AST Encoding

**The decision.** The concrete binary format of the canonical stored form that realizes the
ast-encoding.md frozen contract's requirement that a program's AST have one canonical binary byte
form.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- Each AST has exactly one canonical binary encoding; equal trees encode identically; decoding is the
  inverse (ast-encoding.md §"The Encoding Is A Bijection With One Canonical Byte Form").
- The encoding is a tree of tagged nodes, general and stable across added node kinds (ast-encoding.md
  §"The Encoding Is General And Stable").
- The tree carries comments and documentation (ast-encoding.md §"What The Tree Carries").
- The encoding is versioned (ast-encoding.md §"The Encoding Is Versioned").

This is an ABI/wire-level decision: the encoding fixes the bytes a program is hashed and stored as, so
a change to it is a coordinated change under the constitution's Governance Floors, with a migration
path.

## Choices

- [`binary-sexpr`](./binary-sexpr.md) — a binary s-expression: a minimal tagged tree of atoms and
  lists, deterministic-CBOR-encoded and aligned with the canonical value form. Deliberately simple and
  general, so it stays stable across compiler versions. **The default.**

DEFAULT: binary-sexpr
