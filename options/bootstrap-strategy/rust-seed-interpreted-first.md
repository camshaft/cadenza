# Bootstrap Strategy — Choice: rust-seed-interpreted-first

> **The default choice for the `bootstrap-strategy` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It names the concrete realization of the
> self-hosting requirements the specification states technology-neutrally (constitution XIV;
> bootstrap.md; self-hosting-and-bootstrap.md; build-tool-interface.md §"Derivation By Embedding The
> Reference Interpreter"): the seed language, the derivation modes, and the staging plan.
>
> The seed language and the derivation-mode default are replaceable; the guarantees they must satisfy
> are not.

## The default choices

| Concern | Default | Why |
|---|---|---|
| Seed host language | **Rust** | Compiles to a WebAssembly component via a mature, pinnable toolchain; matches the host's default derivation toolchain, so Cadenza enters as a first-class alternative build tool with no new frozen-root machinery. |
| Initial derivation mode | **interpreted** — embed the reference interpreter over the program's canonical source | A working, hivemind-conformant component exists on day one, before ahead-of-time codegen is complete; the flywheel starts immediately. |
| Maturation derivation mode | **compiled** — ahead-of-time compile source to native component code | An optimization over interpreted derivation; it must agree with the oracle and satisfy every determinism/reproducibility/capability guarantee identically. |
| Reference interpreter authoring | authored in **Cadenza** as soon as the seed can derive it | Makes the single executable semantics a shippable Cadenza artifact and the behavioral oracle, and is the first real step toward self-hosting. |
| Interpreter artifact (seed) | the reference interpreter **compiled to a WebAssembly module** (target **`wasm32-unknown-unknown`**, so its only imports are the granted host capabilities), which **reads an embedded program AST and interprets it at run time** | The derived component actually interprets — it is not a transcript of pre-computed output (build-tool-interface.md §"The Embedded Interpreter Executes In The Component"; `spec/learnings/2026-07-02-decouple-interpreter-wasm-from-host.md`). `wasm32-unknown-unknown` keeps the import set clean, so imports mirror the manifest exactly. |
| Interpreter packaging | the interpreter wasm plus the program's canonical AST embedded as data, bound to one content-addressed component | Keeps an interpreted-derivation output a single verifiable, content-addressed artifact; the same interpreter wasm is reused across programs, which differ only by embedded AST. |
| Host (seed) | a **minimal host, distinct from the interpreter**, that binds only the capability operations a component's manifest enumerates and drives the component | The interpreter and the host are separate concerns (build-tool-interface.md §"The Interpreter And The Host Are Distinct Artifacts"). A host that runs the whole semantics suite through the interpreter component proves the interpreter works (self-hosting-and-bootstrap.md §"The Interpreter Is Proven As A Component Before It Is Iterated On"). |

## The staged plan (line of sight to self-hosting)

1. **Seed interpreter, compiled to wasm + a minimal host.** A reference interpreter written in the
   seed host language (Rust) is compiled to a WebAssembly module (`wasm32-unknown-unknown`) that reads
   an embedded program AST and interprets it. A minimal host, a separate artifact, provides only the
   capability operations a component imports. This is operator-synthesized — the one trusted step the
   flywheel does not itself produce.
2. **Prove the interpreter as a component.** The host runs the whole executable-semantics suite
   *through the interpreter wasm*, reproducing every case the seed realizes. A green suite proves the
   interpreter-as-component works (self-hosting-and-bootstrap.md §"The Interpreter Is Proven As A
   Component Before It Is Iterated On"). Interpreted derivation — binding the interpreter wasm with a
   program's embedded AST into one content-addressed component whose imports mirror its manifest — is
   the first working derivation mode.
3. **Interpreter authored in Cadenza.** With the toolchain proven, the reference interpreter is
   re-authored in Cadenza and derived by the seed, so the single executable semantics becomes a
   shippable Cadenza artifact the flywheel can improve.
4. **Iterate on the log.** Agents extend the Cadenza source of the language and its compiler. Each
   generation is derived by the prior generation, gated (both gates), and activated — a reviewed,
   capability-gated event on the host's log.
5. **Self-hosting.** Reached when the Cadenza compiler is itself authored in Cadenza and derivable
   by the previous Cadenza compiler, with the seed no longer on the critical path.

## The guarantees the strategy must never trade away

- Interpreted and compiled derivation are **behaviorally indistinguishable** and both agree with the
  oracle (build-tool-interface.md §"Derivation By Embedding The Reference Interpreter").
- Every generation's derivation is **reproducible** and every generation is **content-addressed and
  hash-bound** (reproducible-derivation.md).
- "Turning the flywheel" means a generation is **actually derived, gated, and run** — not that the
  events which would accompany a regeneration were emitted (see the learnings on the modeled
  flywheel).
