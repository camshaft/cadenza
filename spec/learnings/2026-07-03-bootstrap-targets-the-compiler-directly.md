# Bootstrap targets the compiler directly; the seed interpreter is native, not wasm

*2026-07-03*

**What happened.** Two simplifications to the bootstrap were adopted during the attended build, after
building a first cut that took the longer path:

1. **The staged path collapsed.** The plan had been: foreign seed → re-author the reference interpreter
   *in Cadenza* → author the compiler in Cadenza → self-host. The middle rung — a Cadenza-authored
   interpreter — was **dropped**. The first Cadenza artifact is now the **compiler**, authored directly
   in Cadenza and derived by running the foreign-language seed interpreter over its source.
2. **The seed interpreter is a native program, not compiled to wasm.** The plan had been to compile the
   seed reference interpreter to a wasm component and derive programs by embedding that interpreter
   component over their ASTs (interpreted derivation), proving the interpreter *as a component* first.
   Instead, the seed interpreter is a **native** program: it reads a Cadenza binary AST, interprets it
   (the behavioral oracle), runs the behavior gate, and — crucially — **runs the Cadenza-authored
   compiler's source**. The compiler's own code generation produces the component bytes.

**Why.** The goal is a self-hosted compiler, and both dropped steps were machinery *between* the seed
and that goal rather than on the path to it:

- A Cadenza-authored *interpreter* has no consumer once the compiler is the target — it is a rung with
  nothing standing on it. (This is the "no throwaway meta-circular interpreter" point that
  [interpreter-first, not compiler-first](./2026-07-02-interpreter-first-not-compiler-first.md) already
  endorsed; that learning is annotated to reflect the collapse.)
- Compiling the *seed interpreter* to wasm confused two separable concerns. Getting a self-hosted
  compiler needs the seed to **run** the compiler's source (native Rust does this directly) and needs
  the **compiler's output** to be a component (that is derivation *output*, produced by the compiler's
  codegen — constitution VI). Neither requires the *seed interpreter itself* to be wasm. Routing the
  seed through `wit-bindgen` / `component new` / AST-embedding was effort spent on a property the goal
  does not need.

Crucially, this is **not** a return to compiler-first (which
[interpreter-first, not compiler-first](./2026-07-02-interpreter-first-not-compiler-first.md) rejected
on constitution IX/XIV grounds): the **reference interpreter remains the single behavioral oracle** —
authored natively, defining behavior, and the thing a compiled program must *agree* with. What changed
is the seed interpreter's *host form* (native, not wasm) and the seed's *first derivation mode*
(compiled codegen to component bytes, oracle-checked), not the existence or authority of the oracle. The
distinction interpreted-derivation guarded — that behavior comes from interpreting, not from a
program-specific transcript ([decouple the interpreter-wasm from the host](./2026-07-02-decouple-interpreter-wasm-from-host.md)) —
is now guarded by the ordinary oracle-agreement check: the compiler's emitted component must exhibit the
same observable behavior as the native reference interpreter over the same input, on every
executable-semantics case, before the generation is promoted.

**The requirement it drove.** Normative edits to the bootstrap specs (attended):
- [bootstrap.md](../bootstrap.md) §"Derivation Modes At Bootstrap" — reframed so the seed's available
  derivation mode is **compiled derivation** (the compiler emits component bytes), oracle-checked;
  interpreted derivation (embed the interpreter over source) is retained as an **optional (`MAY`)** mode
  a generation may offer, no longer the required first mode. §"The Ignition Bar" is unchanged in intent:
  a real Cadenza program is derived to a content-addressed component, run, its imports mirror its
  manifest, it re-derives byte-identically, and it agrees with the oracle.
- [self-hosting-and-bootstrap.md](../capabilities/self-hosting-and-bootstrap.md) §"The Interpreter Is
  Proven Before It Is Relied On" — reframed: the reference interpreter is proven by running the whole
  executable-semantics suite through it **natively** (a green suite proves the oracle), rather than by
  first proving it as a runnable component. §"A Derived Component Agrees With The Oracle" and §"An
  Offered Interpreted Derivation Agrees With A Generated One" replace the former §"Two Modes Produce One
  Behavior" / §"Interpreted Derivation Satisfies Every Guarantee", making the two-mode agreement
  conditional on a generation actually offering interpreted derivation.
- Declared defaults updated in `options/bootstrap-strategy/rust-seed-interpreted-first.md` (seed
  interpreter = native Rust; first derivation mode = compiled codegen to a real component;
  interpreted derivation optional/later) and `options/execution-model/wasm-component-model.md`.

The de-risked real-component recipe ([real components, not a bespoke module model](./2026-07-03-real-components-not-a-bespoke-module-model.md))
still applies — it is how the **compiler's codegen** produces a real component (a core module wrapped by
`wasm-tools component new` whose WIT world matches the manifest); it simply is no longer used to package
the *seed interpreter*.
