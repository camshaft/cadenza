# Bootstrap Strategy — Declared Default

> **What this file is.** The concrete realization of the self-hosting *requirements* the
> specification states technology-neutrally (constitution XIV; bootstrap.md;
> self-hosting-and-bootstrap.md; build-tool-interface.md §"Derivation By Embedding The Reference
> Interpreter"). The spec fixes that a reference interpreter is the oracle, that a compiled
> program's behavior agrees with it, and that a staged path leads from a foreign-language seed to a
> Cadenza-authored compiler; this file names the seed language, the derivation modes, and the
> staging plan.
>
> This is a **declared default**. The seed language and derivation-mode default are replaceable; the
> guarantees they must satisfy are not.

## The default choices

| Concern | Default | Why |
|---|---|---|
| Seed host language | **Rust** | Compiles to a WebAssembly component via a mature, pinnable toolchain; matches the host's default derivation toolchain, so Cadenza enters as a first-class alternative build tool with no new frozen-root machinery. |
| Initial derivation mode | **interpreted** — embed the reference interpreter over the program's canonical source | A working, hivemind-conformant component exists on day one, before ahead-of-time codegen is complete; the flywheel starts immediately. |
| Maturation derivation mode | **compiled** — ahead-of-time compile source to native component code | An optimization over interpreted derivation; it must agree with the oracle and satisfy every determinism/reproducibility/capability guarantee identically. |
| Reference interpreter authoring | authored in **Cadenza** as soon as the seed can derive it | Makes the single executable semantics a shippable Cadenza artifact and the behavioral oracle, and is the first real step toward self-hosting. |
| Interpreter packaging | the interpreter component plus the program's canonical source, bound to one content-addressed component | Keeps an interpreted-derivation output a single verifiable, content-addressed artifact like any other component. |

## The staged plan (line of sight to self-hosting)

1. **Seed.** A compiler written in the seed host language (Rust) derives the first Cadenza toolchain
   to a WebAssembly component. This is operator-synthesized, because nothing yet exists to compile
   Cadenza — the one trusted step the flywheel does not itself produce.
2. **Interpreter as oracle.** The reference interpreter is authored in Cadenza and derived by the
   seed. It becomes the executable semantics' realization and the behavioral oracle. Interpreted
   derivation (embedding this interpreter over source) is the first working derivation mode.
3. **Iterate on the log.** Agents extend the Cadenza source of the language and its compiler. Each
   generation is derived by the prior generation, gated (both gates), and activated — a reviewed,
   capability-gated event on the host's log.
4. **Self-hosting.** Reached when the Cadenza compiler is itself authored in Cadenza and derivable
   by the previous Cadenza compiler, with the seed no longer on the critical path.

## The guarantees the strategy must never trade away

- Interpreted and compiled derivation are **behaviorally indistinguishable** and both agree with the
  oracle (build-tool-interface.md §"Derivation By Embedding The Reference Interpreter").
- Every generation's derivation is **reproducible** and every generation is **content-addressed and
  hash-bound** (reproducible-derivation.md).
- "Turning the flywheel" means a generation is **actually derived, gated, and run** — not that the
  events which would accompany a regeneration were emitted (see the learnings on the modeled
  flywheel).
