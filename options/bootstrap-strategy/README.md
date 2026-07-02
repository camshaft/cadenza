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

- [`rust-seed-interpreted-first`](./rust-seed-interpreted-first.md) — a Rust seed compiler,
  interpreted derivation as the initial default with ahead-of-time compilation as maturation, and the
  four-stage plan to self-hosting. **The default.**

DEFAULT: rust-seed-interpreted-first
