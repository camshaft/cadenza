# Decision — Bootstrap Strategy

**The decision.** The concrete seed language, the derivation modes and their default, and the staged
self-hosting plan that realize the self-hosting requirements the specification states
technology-neutrally (constitution XIV; bootstrap.md; self-hosting-and-bootstrap.md;
build-tool-interface.md §Derivation By Embedding The Reference Interpreter).

**Requirements any choice must satisfy (from the spec — do not weaken):**
- A reference interpreter is the behavioral oracle; a compiled program agrees with it
  (self-hosting-and-bootstrap.md).
- Interpreted and compiled derivation are behaviorally indistinguishable (build-tool-interface.md).
- A staged path leads from a foreign-language seed to a Cadenza-authored compiler, each generation
  derivable by the previous (constitution XIV).

## Choices

- [`rust-seed-interpreted-first`](./rust-seed-interpreted-first.md) — a **native Rust** seed reference
  interpreter (the oracle) that runs the Cadenza compiler's source; the compiler's codegen generates
  real components (compiled derivation), checked against the oracle; interpreted derivation retained as
  an optional/later mode; and the short plan to self-hosting (seed interpreter → Cadenza compiler →
  self-host). **The default.** (The slug's "interpreted-first" is historical — see the choice file's
  naming note.)

DEFAULT: rust-seed-interpreted-first
