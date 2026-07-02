# Decision — AST Encoding

**The decision.** The concrete binary format of the canonical stored form that realizes the
ast-encoding.md frozen contract's requirement that a program's AST have one canonical binary byte
form.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- Each AST has exactly one canonical binary encoding; equal trees encode identically; decoding is the
  inverse (ast-encoding.md §"The Encoding Is A Bijection With One Canonical Byte Form").
- A node is a symbol applied to children, general and stable across added node kinds (ast-encoding.md
  §"The Encoding Is General And Stable").
- The file carries its own symbol prelude, referenced by index, with namespaced and optionally
  versioned symbols in a canonical order (ast-encoding.md §"The Symbol Prelude").
- The tree carries comments and documentation (ast-encoding.md §"What The Tree Carries").
- The container encoding is versioned, and new constructs are new symbols rather than version bumps
  (ast-encoding.md §"Versioning").

This is an ABI/wire-level decision: the encoding fixes the bytes a program is hashed and stored as, so
a change to it is a coordinated change under the constitution's Governance Floors, with a migration
path.

## Choices

- [`binary-sexpr`](./binary-sexpr.md) — a binary s-expression: nodes are symbols (referenced by index
  into a self-contained per-file prelude) applied to children, deterministic-CBOR-encoded and aligned
  with the canonical value form. Deliberately simple and general, so it stays stable across compiler
  versions. **The default.**

DEFAULT: binary-sexpr
