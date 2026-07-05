# Bootstrap Strategy — Choice: rust-seed-interpreted-first

> **The default choice for the `bootstrap-strategy` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It names the concrete realization of the
> self-hosting requirements the specification states technology-neutrally (constitution XIV;
> bootstrap.md; self-hosting-and-bootstrap.md): the seed language, the seed's host form, the derivation
> modes, and the staging plan.
>
> **Naming note.** The slug `…-interpreted-first` is now **doubly historical**. The *seed* is a
> **native Rust reference *compiler*** (`cdz-rustc`) that lowers a Cadenza program's canonical AST to a
> real WebAssembly component and runs it; there is no reference interpreter on the critical path. The
> behavioral **oracle is the conformance corpus** (the recorded executable semantics), and the
> independence of the judgment is supplied by **two implementations of the compiler** — the Rust seed
> and the Cadenza-authored `compiler.cdz` — that must agree. A reference interpreter is retained only
> as an **optional independent oracle**, never as a runtime. See
> [spec/learnings/2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md](../../spec/learnings/2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md),
> which supersedes the seed-shape half of
> [2026-07-03-bootstrap-targets-the-compiler-directly.md](../../spec/learnings/2026-07-03-bootstrap-targets-the-compiler-directly.md).
> The slug is kept only to avoid churning the `DEFAULT:` linkage; it is a name, not a claim.
>
> The seed language and the derivation-mode default are replaceable; the guarantees they must satisfy
> are not.

## The default choices

| Concern | Default | Why |
|---|---|---|
| Seed host language | **Rust** | Compiles to a WebAssembly component via a mature, pinnable toolchain; matches the host's default derivation toolchain, so Cadenza enters as a first-class alternative build tool with no new frozen-root machinery. |
| Seed artifact | a **native Rust reference *compiler* (`cdz-rustc`)** (NOT compiled to wasm) | The seed's job is to lower Cadenza source to a component and run it, and to compile the Cadenza-authored compiler's source; neither needs the seed itself to be wasm. There is no reference interpreter — behavior is observed by running compiled components, not by a foreign-language tree-walk. |
| Behavioral oracle | the **conformance corpus** (the recorded executable semantics) | Principle IX already holds the corpus as the single source of truth for behavior; its recorded, reviewed values are the authority a compiled program must agree with — no interpreter is needed to define behavior. |
| Independence of the judgment | **two compiler implementations** — `cdz-rustc` (Rust) and `compiler.cdz` (Cadenza) — that MUST agree on every realized program | Replaces the interpreter-vs-compiler differential with a compiler-vs-compiler one; two independent lowerings of one spec catch a divergence a single implementation cannot self-detect (constitution XIV). |
| Reference interpreter | **optional independent oracle only**; never a runtime, never on the bootstrap path | Retained as a `MAY` (bootstrap.md §"A Reference Interpreter Is An Optional Independent Oracle") for extra cross-checking; the language's one runtime is WebAssembly. |
| Seed derivation mode | **compiled** — `cdz-rustc` lowers the AST to a **complete** component binary (a real component whose WIT world matches the manifest) and runs it | A runnable component exists from the first foreign-language artifact; every program runs as a component, so no separate execution engine defines behavior alongside the compiler. |
| First Cadenza artifact | the **Cadenza-authored compiler (`compiler.cdz`)**, compiled to a component by `cdz-rustc` | The bootstrap target (self-hosting-and-bootstrap.md §"Each Generation Is Derived By The Previous"); a language port of the same AST→wasm lowering `cdz-rustc` performs, so the two share a spine and cross-check. |
| Self-hosted-component packaging | `compiler.cdz`'s codegen emits the **complete** real, content-addressed **component** binary — core module plus the component-model envelope — as a `Bytes` value whose WIT world declares the granted capabilities; a `wasm-tools`-class validator is used only as an out-of-band **oracle** of well-formedness, never as a step that produces or completes the bytes | The self-hosted artifact's bytes are a function of the Cadenza compiler alone, so self-hosting is a clean fixpoint (`spec/learnings/2026-07-03-the-compiler-emits-the-whole-component.md`); `cdz-rustc` emits the same real-component shape (`spec/learnings/2026-07-03-real-components-not-a-bespoke-module-model.md`). |
| Host (seed) | a **minimal host** that binds only the capability operations a component's manifest enumerates and drives a *compiled* component | Distinct from the compiler (build-tool-interface.md §"The Component And The Host Are Distinct Artifacts"); used to run and observe a compiled component at the ignition bar and in both gates. |

## The staged plan (line of sight to self-hosting)

The staging is deliberately **short**: seed compiler (`cdz-rustc`) → Cadenza-authored compiler
(`compiler.cdz`) → self-hosting. Both rungs are compilers; the second is a language port of the
first, so they share a spine and cross-check each other. There is no reference interpreter on the
path.

1. **Seed reference compiler, native + a minimal host.** `cdz-rustc`, written in the seed host
   language (Rust), runs as a **native program**: it reads a program's canonical binary AST, lowers it
   to a complete real WebAssembly component (core module + component-model envelope, WIT world matching
   the manifest), and runs it via the minimal host. It is NOT compiled to wasm — its role is to compile
   Cadenza and to compile `compiler.cdz`'s source. The minimal host, a separate artifact, provides the
   capability operations a compiled component imports. This is operator-synthesized — the one trusted
   step the flywheel does not itself produce.
2. **Prove `cdz-rustc` against the corpus (the oracle).** The seed compiles every realized
   executable-semantics case to a component, runs it, and confirms its observable behavior equals the
   recorded result. A green suite proves the seed compiler before any generation is judged against it
   (self-hosting-and-bootstrap.md §"Both Compilers Are Proven Against The Corpus Before They Are Relied
   On").
3. **Compiler authored in Cadenza.** `compiler.cdz` is authored **directly in Cadenza** as a port of
   the same AST→wasm lowering, and compiled to a component by `cdz-rustc`; its codegen emits the
   complete real component binary as a `Bytes` value (whose WIT world matches the manifest), with no
   wrapping tool in the byte path. It must reproduce the recorded semantics AND agree with `cdz-rustc`
   on every program it can compile. It needs the language's first-class functions, sum types, records,
   recursion, lists, strings, and the `Bytes` value form it builds the wasm bytes up as.
4. **Iterate on the log.** Agents extend the Cadenza source of the language and both compilers. Each
   generation is derived by the prior generation, gated (behavior gate against the corpus + the
   two-compiler differential), and activated — a reviewed, capability-gated event on the host's log.
5. **Self-hosting.** Reached when `compiler.cdz` compiles its own source to a component byte-identical
   to the one `cdz-rustc` produces from it, with the Rust seed no longer on the critical path.

## The guarantees the strategy must never trade away

- The two compiler implementations are **behaviorally indistinguishable** on every realized program
  and both agree with the recorded semantics (self-hosting-and-bootstrap.md §"The Two Compilers Agree
  On Every Realized Program").
- Every generation's derivation is **reproducible** and every generation is **content-addressed and
  hash-bound** (reproducible-derivation.md).
- Every program runs as a **WebAssembly component**; no separately-maintained execution engine defines
  behavior alongside the compiler.
- "Turning the flywheel" means a generation is **actually derived, gated, and run** — not that the
  events which would accompany a regeneration were emitted (see the learnings on the modeled
  flywheel).
