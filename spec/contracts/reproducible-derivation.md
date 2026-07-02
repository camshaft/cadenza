# Frozen Contract — Reproducible Derivation

> **FROZEN CONTRACT.** This document pins that compiling is a pure function of canonical source
> and pinned toolchain, and that the compiler strips or normalizes anything that would otherwise
> vary between builds. It applies at both levels at which Cadenza is reproduced: the compiler
> itself is derived reproducibly, and the compiler derives its output reproducibly. It is versioned
> and changed only by the coordinated act described in the constitution's Governance Floors. Its
> requirements realize [Core Principle I](../../constitution.md) and
> [Core Principle II](../../constitution.md) and trace to [overview §7](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. This contract names the reproducibility property, not a concrete toolchain; the
> pinned toolchain identity is recorded at the declared-default location.

## Purpose And Scope

A component's identity is a hash over its bytes, and its legitimacy is that anyone can re-derive it
from source and confirm the hash. That only works if derivation is a pure function of the canonical
source and the pinned toolchain — if the same inputs always produce the same bytes. This contract
pins that purity and the normalization that removes incidental variation. It applies equally to the
compiler deriving a user's program and to the toolchain deriving the compiler itself. It does not
pin the concrete toolchain, which is a declared default.

## Derivation Is A Pure Function

### Derivation Is A Function Of Source And Toolchain

Deriving the same canonical source with the same pinned toolchain MUST produce byte-identical component output.

The compiler MUST record the identity of the toolchain that produced a component alongside that component.

### Codegen Order Is Source-Determined

The order in which the compiler emits definitions, data, and interface entries MUST be a deterministic function of the source.

The compiler MUST NOT let filesystem enumeration order or nondeterministic collection iteration affect the order of its output.

## Provenance Is Removed

### Provenance Is Stripped Or Normalized

The compiler MUST remove or normalize any embedded producer string, build path, or timestamp that would otherwise vary between builds of the same source.

The compiler MUST NOT embed into its output any value derived from the build host's environment that is not a function of the source and the pinned toolchain.

## Verification By Third Parties

### Anyone May Re-Derive And Verify

A third party MUST be able to re-derive a component from its canonical source and the recorded toolchain identity and obtain a component whose hash matches the one bound to that source.

A verifier MUST be able to confirm that a component matches its source without trusting the party that originally derived it.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract MUST be additive with respect to already-derived components, or else carry an explicit version increment and a stated migration path.
