# Toolchain — Declared Default

> **What this file is.** The pinned toolchain identity that realizes reproducible-derivation.md's
> requirement that "the same canonical source with the same pinned toolchain produces a
> byte-identical component" and that "the compiler records the identity of the toolchain that
> produced a component alongside that component." This is the pin that makes reproducibility
> checkable at *both* levels: the compiler reproducing a user's program, and the toolchain
> reproducing the compiler itself.
>
> This is a **declared default**. Changing the pinned toolchain in a way that alters the bytes
> produced from unchanged source is a coordinated change under reproducible-derivation.md §"Toolchain
> Change Discipline"-style handling.

## The two levels this pins

- **Level 1 — reproducing the compiler.** Cadenza-the-compiler is itself derived from its source by
  a pinned host-language toolchain to a content-addressed component. This file records the identity
  of that host-language toolchain and the compiler's own component hash, so a third party can
  re-derive the compiler and confirm it.
- **Level 2 — reproducing a user's program.** The compiler records, alongside every component it
  produces, the identity of the compiler generation (its own component hash) that produced it, so a
  program's component is re-derivable by naming the exact compiler that made it.

## The default pins

| Pin | Default |
|---|---|
| Seed host language | **Rust** (see [`bootstrap-strategy.md`](./bootstrap-strategy.md)) |
| Host-language toolchain identity | the pinned compiler version and target, recorded as a content hash over the toolchain manifest |
| Compiler component identity | the SHA-256 content hash of the derived Cadenza compiler component |
| Toolchain-identity record | recorded alongside each produced component as `{ compiler-component-hash, host-toolchain-hash }` |
| Provenance normalization | producer strings, build paths, and timestamps stripped or normalized per reproducible-derivation.md §"Provenance Is Stripped Or Normalized" |

## Why record identity rather than embed it

A component must be byte-identical across builds, so build-varying provenance is stripped from the
component itself. The toolchain identity that *produced* it is recorded **alongside** the component
(in the module declaration that carries it), not embedded in its bytes — so the component stays
reproducible while remaining re-derivable by anyone who names the recorded toolchain.
