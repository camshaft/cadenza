# Cadenza

A programming language **for AI agents** — easy for an agent to write and read, easy for a human to
read, easy to verify, and compiled to sandboxed WebAssembly components.

Cadenza is **spec-driven**: the specifications are the only durable artifact, and the compiler is a
disposable, regenerable projection of them. A defect is fixed in the source or the spec and
recompiled — never by patching a live compiler.

## The idea

- **The source is the truth; the component is derived.** A program's meaning lives in its source; its
  runnable form is a content-addressed WebAssembly component, a reproducible function of that source.
- **Determinism and capability-safety are the floor, not features.** Same source + toolchain → byte-identical
  output; same input → identical results on every runtime; a component reaches only what its manifest
  declares; execution is bounded by a deterministic fuel measure. By construction, not convention.
- **Verification is progressive.** A program compiles when the core guarantees hold (types, determinism,
  capability-safety). Contracts, refinement types, effect tracking, and proofs are opt-in layers added
  as a program matures — and adding one never changes what the program already means.
- **The language builds itself.** The executable-semantics corpus is the behavioral oracle;
  independence comes from two compilers — a foreign-language seed and the Cadenza-authored one — that
  must agree. The seed derives the first Cadenza toolchain; each later generation is authored in
  Cadenza and derived by the one before it.

The target is a pool of agents over a durable event log: behavior is published as source + a capability
manifest, and its runnable form is a sandboxed, content-addressed component. The system's frozen root
only loads, verifies, and runs components — it has no compiler. Cadenza is the replaceable,
capability-gated build tool that turns source into those components, and is itself one.

## Working in the repo

The seed toolchain is a Rust workspace under `implementation/seed/`, driven entirely by **`cargo xtask`**
(the one interface — see the `seed-workspace` skill for the full tour).

```sh
cargo xtask setup                 # FIRST, in a fresh clone/worktree: links .claude/{skills,commands}
cargo xtask run prog.cdz          # compile-and-run a program, print the result
cargo xtask gate                  # run the executable-semantics corpus, grade every case
cargo xtask check                 # omnibus health: build + test + clippy + wasm runtime + gate
```

`.claude/` is gitignored, so `setup` wires this checkout up to the tracked `skills/` and `commands/`.

## Layout

- `constitution.md` — the non-negotiable invariants, as normative requirements.
- `spec/overview.md` — the architecture and intent; the north star every requirement traces to.
- `spec/contracts/` — **frozen** byte- and ABI-level forms, honored across every regeneration.
- `spec/capabilities/` — the behavior of the language and its compiler, implementation-free.
- `spec/semantics/` — the one executable-semantics corpus: the source of truth for what every construct *does*.
- `spec/bootstrap.md` — the seed toolchain and the line to self-hosting.
- `spec/learnings/` — dated post-mortems that drove this design.
- `options/` — open **decisions**, each a directory of candidate **choices** with a declared default.
- `implementation/seed/` — the Rust seed workspace (front end, compiler, runtime, host); `xtask/` drives it.
- `.duvet/` — the conformance gate: every normative sentence mapped to the code and tests that satisfy it.

## The conformance gate

Every normative statement is a single RFC-2119 sentence under a stable heading, so
[duvet](https://github.com/awslabs/duvet) can extract it — a requirement's identity is `(spec file,
section, quoted sentence)`, so changing wording flags every citation that no longer matches. A
regenerated compiler cites the requirements it satisfies; a generation in which any load-bearing
requirement lacks both an implementation and a test citation is not promoted. Behavior carries a second
gate: every case in `spec/semantics/` must execute to its recorded output (`cargo xtask gate`).

## Status

Clean-room specification, in authoring, with the seed toolchain climbing the corpus. The compiler is a
regenerable projection. Prior generations — a tree-walking interpreter, a Salsa incremental core, a
declarative meta-compiler, a K-framework reference — are historical prior art; these specs are
standalone and derive the language from first principles.
