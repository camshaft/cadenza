# Bootstrap Strategy — Choice: rust-seed-interpreted-first

> **The default choice for the `bootstrap-strategy` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It names the concrete realization of the
> self-hosting requirements the specification states technology-neutrally (constitution XIV;
> bootstrap.md; self-hosting-and-bootstrap.md): the seed language, the seed's host form, the derivation
> modes, and the staging plan.
>
> **Naming note.** The slug `…-interpreted-first` is historical. The *seed* is now a **native Rust
> reference interpreter that is the oracle**, and the seed's derivation mode is **compiled** — the
> Cadenza compiler's codegen generates a real component, checked against the oracle. Interpreted
> derivation (embedding the interpreter over a program's AST) is retained as an **optional/later** mode,
> not the seed's first mode. See
> [spec/learnings/2026-07-03-bootstrap-targets-the-compiler-directly.md](../../spec/learnings/2026-07-03-bootstrap-targets-the-compiler-directly.md).
> The slug is kept to avoid churning the `DEFAULT:` linkage; it is only a name.
>
> The seed language and the derivation-mode default are replaceable; the guarantees they must satisfy
> are not.

## The default choices

| Concern | Default | Why |
|---|---|---|
| Seed host language | **Rust** | Compiles to a WebAssembly component via a mature, pinnable toolchain; matches the host's default derivation toolchain, so Cadenza enters as a first-class alternative build tool with no new frozen-root machinery. |
| Seed interpreter host form | **native Rust program** (NOT compiled to wasm) | The seed's job is to define behavior (the oracle) and to RUN the Cadenza compiler's source; neither needs the seed itself to be wasm. Making a derived *program* a component is the compiler's codegen concern, not a property of the seed (`spec/learnings/2026-07-03-bootstrap-targets-the-compiler-directly.md`). |
| Seed derivation mode | **compiled** — the Cadenza compiler's codegen generates the component (a core module wrapped by `wasm-tools component new` into a real component whose WIT world matches the manifest), checked against the oracle | A runnable component exists without embedding the interpreter in it; the derived component must agree with the native reference interpreter on every realized executable-semantics case before promotion. |
| Interpreted derivation | **optional / later** — a generation MAY also embed the reference interpreter over a program's AST | Retained as an alternative mode (bootstrap.md §"Interpreted Derivation Is An Optional Mode"), not the seed's required first mode; it must agree with the oracle and satisfy every guarantee identically when offered. |
| Reference interpreter authoring | authored in the **foreign seed language (Rust)** and left there; it is NOT re-authored in Cadenza | The staging is collapsed: the first Cadenza artifact is the **compiler**, derived by running the seed interpreter over its source, not a Cadenza-authored interpreter. Dropping that rung removes indirection and gets to the artifact we want (a compiler) sooner. |
| First Cadenza artifact | the **Cadenza-authored compiler**, derived by the seed interpreter | This is the bootstrap target (self-hosting-and-bootstrap.md §"Each Generation Is Derived By The Previous"); it needs the language's first-class functions, sum types, records, recursion, lists, and strings, all realized by the seed. |
| Seed interpreter artifact | a **native Rust program**: reads a Cadenza binary AST, interprets it to observable behavior (the oracle), runs the behavior gate directly, and runs the Cadenza compiler's source | Proving the oracle is running the whole semantics suite through the native interpreter — no wasm packaging of the interpreter needed (self-hosting-and-bootstrap.md §"The Interpreter Is Proven Before It Is Relied On"). |
| Generated-component packaging | the **compiler's codegen** emits a core module and wraps it with `wasm-tools component new` into one real, content-addressed **component** whose WIT world declares the program's granted capabilities | The end-goal artifact is a real component whose world matches the manifest (`spec/learnings/2026-07-03-real-components-not-a-bespoke-module-model.md`); this is derivation output produced by the compiler, not a property of the seed interpreter. |
| Host (seed) | a **minimal host** that binds only the capability operations a component's manifest enumerates and drives a *derived* component | Distinct from the interpreter (build-tool-interface.md §"The Interpreter And The Host Are Distinct Artifacts"); used to run and observe a derived component at the ignition bar. |

## The staged plan (line of sight to self-hosting)

The staging is deliberately **short**: seed interpreter → Cadenza-authored compiler → self-hosting.
There is no intermediate "re-author the interpreter in Cadenza" rung — that indirection is dropped;
the first Cadenza artifact is the compiler, which is what we ultimately want.

1. **Seed reference interpreter, native + a minimal host.** A reference interpreter written in the seed
   host language (Rust) runs as a **native program**: it reads a program's canonical binary AST and
   interprets it to observable behavior. It is NOT compiled to wasm — its role is to define behavior
   (the oracle) and to run the Cadenza compiler's source. A minimal host, a separate artifact, provides
   the capability operations a *derived* component imports and drives that component. This is
   operator-synthesized — the one trusted step the flywheel does not itself produce.
2. **Prove the interpreter (the oracle).** The seed runs the whole executable-semantics suite through
   the native interpreter, reproducing every case it realizes. A green suite proves the oracle before
   any generation is judged against it (self-hosting-and-bootstrap.md §"The Interpreter Is Proven
   Before It Is Relied On").
3. **Compiler authored in Cadenza.** The Cadenza compiler is authored **directly in Cadenza** and
   derived by running the seed interpreter over its source; the compiler's codegen generates a real
   component (a core module wrapped by `wasm-tools component new` whose WIT world matches the manifest),
   whose observable behavior must agree with the oracle. This is the first Cadenza artifact; it needs
   the language's first-class functions, sum types, records, recursion, lists, and strings (all realized
   by the seed) to be expressible. No Cadenza-authored interpreter stands between the seed and the
   compiler.
4. **Iterate on the log.** Agents extend the Cadenza source of the language and its compiler. Each
   generation is derived by the prior generation, gated (both gates), and activated — a reviewed,
   capability-gated event on the host's log.
5. **Self-hosting.** Reached when the Cadenza compiler is itself derivable by the previous Cadenza
   compiler, with the seed interpreter no longer on the critical path.

## The guarantees the strategy must never trade away

- Interpreted and compiled derivation are **behaviorally indistinguishable** and both agree with the
  oracle (build-tool-interface.md §"Derivation By Embedding The Reference Interpreter").
- Every generation's derivation is **reproducible** and every generation is **content-addressed and
  hash-bound** (reproducible-derivation.md).
- "Turning the flywheel" means a generation is **actually derived, gated, and run** — not that the
  events which would accompany a regeneration were emitted (see the learnings on the modeled
  flywheel).
