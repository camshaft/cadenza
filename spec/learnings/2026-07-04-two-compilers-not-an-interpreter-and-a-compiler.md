# Two compilers, not an interpreter and a compiler; the runtime is wasm

*2026-07-04*

**What happened.** The bootstrap is being restructured. The seed stops being a *reference
interpreter* (a native tree-walker that defines behavior and runs the Cadenza compiler's source)
and becomes a *reference compiler* (`cdz-rustc`): a native Rust program that lowers a Cadenza
program's canonical AST to a real WebAssembly component and runs it on the host. The
Cadenza-authored compiler (`compiler.cdz`) stays. So the toolchain now has **two implementations of
one compiler** — one expressed in Rust, one in Cadenza — rather than one interpreter and one
compiler.

**Why.** Three things converged, all pointing the same way:

1. **The goal is a compiler, and the runtime is wasm.** The tree-walking interpreter reinvents a
   runtime that wasm already provides — integer overflow trapping, the bounded-resource halt (stack
   exhaustion → trap), arithmetic, the value model. Leaning into wasm as *the* runtime and letting
   the Rust side *compile and run a component* removes a whole reimplemented execution engine. The
   seed should compile Cadenza to a component and run it, not run it directly.

2. **An interpreter and a compiler share almost nothing.** Every construct was built twice with no
   cross-pollination: once as interpreter `eval`, once as Cadenza codegen. The two artifacts have
   different shapes, so a feature in one gives no leverage on the other. Two *compilers* share a
   spine — AST → type/kind synthesis → wasm lowering → component envelope. Expressing that spine in
   Rust and in Cadenza is a **language port**: the same concepts, twice, cross-checking each other.
   It is a translation, not two designs.

3. **Codegen was being grown blind.** Growing `compiler.cdz` alone means authoring wasm-emitting
   codegen in a dynamically-interpreted Cadenza with no debugger and no reference lowering to diff
   against — every byte hand-verified through the differential gate. A Rust reference compiler makes
   the same lowering exist in a language with tooling, and turns "is my Cadenza codegen right?" into
   "does it match the Rust lowering?" — a diff, not an audit.

**What this costs — stated plainly.** The bootstrap regress that the interpreter resolved comes
back. You cannot *run* a Cadenza-authored compiler without something that can execute it; the
interpreter was that something. With it gone, **`cdz-rustc` must eventually compile `compiler.cdz`
itself** — which means the Rust seed grows into a *full* Cadenza→wasm compiler (first-class
closures, sum types, lists and strings in linear memory, pattern matching), a strictly bigger
machine than a tree-walker. This is accepted: it is the machine the project needs anyway, and
building it once in Rust (with tooling) then porting to Cadenza is cheaper than growing it once,
blind, in Cadenza.

**What replaces the interpreter-as-oracle.** The independent cross-check the interpreter provided
does not vanish; it changes shape.

- **The corpus is the oracle.** Principle IX already holds the executable-semantics corpus as *the*
  single source of truth for behavior; each case records its observable result. That recorded result
  — human-authored and reviewed — is the authority a compiled program must agree with. The behavior
  gate becomes: compile each realized case, run the component, confirm its observable behavior equals
  the recorded result.
- **Two compilers are the differential.** Where the differential gate was interpreter-vs-compiler,
  it becomes stage-0 (`cdz-rustc`) vs stage-1 (`compiler.cdz` compiled by `cdz-rustc`): two
  independent lowerings of the one spec that must produce behaviorally-identical — ultimately
  byte-identical — components. Divergence between two independent implementations is the signal the
  interpreter used to give.
- **Self-hosting is unchanged in spirit.** Reached when the Cadenza compiler compiles its own source
  to a component byte-identical to the one `cdz-rustc` produces, with the Rust seed off the critical
  path.

**The tradeoff we are accepting.** Interpreter-vs-compiler compared two artifacts of *different
shape* — a strong check, because a bug would have to occur identically in a tree-walk and in
codegen to hide. Compiler-vs-compiler compares two artifacts of the *same* shape, so a shared design
error (in the spec, or copied in the port) can hide in both; the corpus's recorded values are the
backstop against that. Net: we trade one axis of independence (different execution strategies) for
tighter concept-sharing and a wasm-native runtime, and lean harder on the corpus as the authority.
This is a bet that the corpus + two same-shape compilers catch more, sooner, than one interpreter +
one blind compiler did — and it is exactly the kind of bet the flywheel exists to test and record.

**The requirements it drove.**
- [constitution.md](../../constitution.md) Core Principle XIV amended: the oracle is the executable
  semantics as recorded by the corpus rather than necessarily a reference interpreter; a reference
  interpreter becomes an optional (`MAY`) realization; a compiled program must agree with the
  executable semantics over the same input; independence is supplied by two implementations of the
  compiler that must agree. Recorded under Amendment Discipline with a version increment.
- [bootstrap.md](../bootstrap.md) §"The Reference Interpreter As Oracle" / §"Derivation Modes At
  Bootstrap" reframed: the seed is a reference *compiler*; the oracle is the recorded semantics; the
  seed compiles the Cadenza compiler's source rather than interpreting it.
- [self-hosting-and-bootstrap.md](../capabilities/self-hosting-and-bootstrap.md) §"The Oracle" /
  §"The Staged Path" reframed to corpus-as-oracle and a two-compiler differential.
- [options/bootstrap-strategy/rust-seed-interpreted-first.md](../../options/bootstrap-strategy/rust-seed-interpreted-first.md)
  default updated: seed host form = native Rust **compiler**; oracle = corpus; interpreter =
  optional. (The slug is now doubly historical.)

This supersedes the seed-shape half of
[bootstrap targets the compiler directly](./2026-07-03-bootstrap-targets-the-compiler-directly.md):
that learning already dropped the Cadenza-authored *interpreter* rung and made the seed native and
compiled-first; this goes one step further and drops the *seed* interpreter too, making the seed a
compiler and the corpus the oracle. The earlier learning's core claim — the target is a compiler, not
an interpreter — is affirmed, not reversed.
