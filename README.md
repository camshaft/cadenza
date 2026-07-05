# Cadenza

A programming language built **for AI agents** — easy for an agent to write and read,
easy for a human to read, easy to verify properties of, and compiled to sandboxed
WebAssembly components.

This repository is **spec-driven**: the specifications are the only durable artifact, and
the compiler is a disposable, regenerable projection of them. When a design turns out
wrong, we rework the spec and regenerate rather than patch a live compiler. This is safe by
construction — a program's meaning lives in its source, the runnable form is a
content-addressed component re-derivable from that source, and the compiler that derives it
can be rebuilt from these specs at any time.

## The idea, in four turns

1. **The source is the truth; the component is derived.** A program's meaning lives in its
   source. Its runnable form is a content-addressed WebAssembly component that is a
   reproducible function of that source — a rebuildable artifact, never the authority. A
   defect is fixed in the source or the spec and recompiled, never by editing a component.
2. **Determinism and capability-safety are not features; they are the floor.** The same
   source and toolchain produce byte-identical output; the same input produces byte-identical
   results on every runtime; a component reaches nothing its manifest did not declare;
   execution is bounded by a deterministic fuel measure, never by wall-clock time. These hold
   by construction, not by convention.
3. **Verification is progressive.** A program compiles when only the core guarantees — types,
   determinism, capability-safety — hold. Contracts, refinement types, effect tracking, and
   machine-checked proofs are opt-in layers a program adds as it matures, and adding a layer
   never changes what a program already means.
4. **The language can build itself.** The executable-semantics corpus is the single source of
   truth for behavior and the behavioral oracle; independence comes from two implementations of
   the compiler — a foreign-language seed and the Cadenza-authored compiler — that must agree. The
   seed compiler derives the first Cadenza toolchain to a component; from there each generation of
   the language is authored in Cadenza and derived by the one before it — the flywheel.

## Why Cadenza exists

Cadenza is a source language and derivation tool for a target system — a pool of agents over
one durable event log, where behavior itself is data published on the log as source plus a
capability manifest, and the runnable form of that behavior is a sandboxed, content-addressed
component derived from the source. That system's frozen root only loads, verifies, and runs
components; it contains no compiler. Cadenza is the replaceable, capability-gated build tool
that turns source into components the target can run — and is itself such a
component. The constraints the target places on the behavior it runs — determinism,
capability-safety, bounded termination, reproducible derivation, content-addressing — are the
constraints this language is designed around.

## Layout

- `constitution.md` — the non-negotiable invariants, as normative requirements.
- `spec/overview.md` — the architecture and intent; the north star every requirement traces to.
- `spec/glossary.md` — the controlled vocabulary.
- `spec/contracts/` — **frozen** byte- and ABI-level forms, honored across every regeneration
  of the compiler.
- `spec/capabilities/` — the behavior of the language and its compiler, implementation-free.
- `spec/semantics/` — the one executable semantics corpus: the single source of truth for what
  every construct *does*, gated by execution.
- `spec/bootstrap.md` — the seed toolchain and the line of sight to self-hosting.
- `spec/learnings/` — dated post-mortems that drove this design.
- `options/` — the open **decisions** (component format, engine, hashing, numeric model, code shape,
  …), each a directory of candidate **choices** with a **declared default** — accept the default,
  pick another choice, or author your own for that one decision.
- `templates/` — authoring templates for contracts, capability specs, learnings, and semantics
  cases.
- `.duvet/` — the conformance gate: every normative sentence extracted and mapped to the code
  and tests that satisfy it.

## The conformance gate

Every normative statement is a single RFC-2119 sentence under a stable heading, so
[duvet](https://github.com/awslabs/duvet) can extract it. A requirement's identity is
`(spec file, section, quoted sentence)` — there are no invented identifiers, and changing a
sentence's wording flags every citation that no longer matches it. A regenerated compiler
cites the requirements it satisfies; a generation in which any load-bearing requirement lacks
both an implementation citation and a test citation is not promoted. Behavior carries a second
gate: every case in `spec/semantics/` must execute to its recorded output.

## Status

Clean-room specification, in authoring. The compiler is a regenerable projection and is not
committed to this branch. Prior generations of Cadenza — a tree-walking interpreter, a Salsa
incremental core, a declarative meta-compiler, and a K-framework reference — are historical
prior art; the specs here are standalone and derive the language from first principles for the
agent-first, verifiable, WebAssembly-component north star.
