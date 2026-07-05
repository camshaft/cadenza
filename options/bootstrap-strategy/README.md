# Decision — Bootstrap Strategy

**The decision.** The concrete seed language, the derivation path, and the staged
self-hosting plan that realize the self-hosting requirements the specification states
technology-neutrally (constitution XIV; bootstrap.md; self-hosting-and-bootstrap.md;
build-tool-interface.md §Derivation By Compilation).

**Requirements any choice must satisfy (from the spec — do not weaken):**
- The conformance corpus (recorded executable semantics) is the behavioral oracle; a compiled program
  agrees with it (self-hosting-and-bootstrap.md; constitution XIV).
- Two independent compiler implementations must agree on the observable behavior of every program a
  generation realizes (self-hosting-and-bootstrap.md).
- A staged path leads from a foreign-language seed to a Cadenza-authored compiler, each generation
  derivable by the previous (constitution XIV).

## Choices

- [`rust-seed-interpreted-first`](./rust-seed-interpreted-first.md) — a **native Rust** seed reference
  **compiler** (`cdz-rustc`) that lowers Cadenza source to a real component and runs it; the
  **conformance corpus** is the oracle; independence comes from **two compiler implementations**
  (`cdz-rustc` + `compiler.cdz`) that must agree; a reference interpreter is retained only as an
  optional oracle; and the short plan to self-hosting (seed compiler → Cadenza compiler →
  self-host). **The default.** (The slug's "interpreted-first" is historical — see the choice file's
  naming note.)

DEFAULT: rust-seed-interpreted-first
