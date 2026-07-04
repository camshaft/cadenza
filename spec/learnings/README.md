# Learnings

Dated post-mortems that drove this specification. Each entry follows the format in
[`templates/learning.md`](../../templates/learning.md): **What happened / Why / The requirement it
drove**. Learnings are descriptive, not normative — they carry no RFC-2119 requirements and are not
listed in the requirement gate. They are the one place a specification artifact may name a prior
prototype or a concrete implementation, because a learning is historical reference for why a durable
change exists.

The learnings here are the reasons this clean-room specification is shaped as it is. Earlier
generations of Cadenza taught these lessons the expensive way; the specification is the response.

- [The compiler core was restarted four times](./2026-07-02-compiler-core-restarted-four-times.md) —
  why the specification, not the compiler, is the durable artifact.
- [Component output never materialized](./2026-07-02-component-output-never-materialized.md) — why the
  component ABI and determinism are frozen contracts written before the capabilities.
- [Four parallel semantics drifted](./2026-07-02-parallel-semantics-drifted.md) — why there is one
  executable semantics, gated by execution.
- [Multiple front-ends diluted one surface](./2026-07-02-multiple-frontends-diluted-one-surface.md) —
  why there is one canonical representation with decoupled displays.
- [Verification was baked through the tree](./2026-07-02-verification-baked-through-the-tree.md) — why
  verification is progressive and meaning-preserving.
- [There was no line of sight to self-hosting](./2026-07-02-no-line-of-sight-to-self-hosting.md) — why
  the reference interpreter is the oracle and the seam to the flywheel.
- [A modeled subsystem passes a shape check](./2026-07-02-a-modeled-subsystem-passes-a-shape-check.md)
  — why behavior requirements are discharged by execution and every requirement binds to an enforcing
  line. (Adopted from the host project's own hard-won lesson.)
- [The seed is a dynamic interpreter](./2026-07-02-seed-is-a-dynamic-interpreter.md) — why the seed
  generation defers static typing and realizes evaluation dynamically to get the flywheel turning, and
  the Core Principle VII bootstrap carve-out that records the amendment.
- [The ignition path is de-risked](./2026-07-02-ignition-path-de-risked.md) — the two Phase-2 spikes:
  duvet's quoted-sentence gate works for Rust (but exits 0 on citation errors), and the
  source→derive→run→re-derive path is real and byte-reproducible in this environment.
- [Decouple the interpreter-wasm from the host](./2026-07-02-decouple-interpreter-wasm-from-host.md) —
  interpreted derivation embeds the interpreter *component* over the program's AST (so the component
  actually interprets, not replays a transcript); the host providing capability functions is a
  separate minimal artifact. Avoids the modeled-derivation trap.
- [Bootstrap is interpreter-first, not compiler-first](./2026-07-02-interpreter-first-not-compiler-first.md)
  — why a compiler-first self-hosting proposal was considered and rejected (it has no behavioral
  oracle and revives the meaning-in-the-compiler failure), while its compatible ideas were adopted;
  switching would be a deliberate constitution IX/XIV amendment.
- [An effect-only program had no normal-termination value](./2026-07-02-effect-only-programs-need-a-unit-value.md)
  — why a Unit value was pinned (additively) so event-emitting programs carry a definite terminal
  condition; surfaced by four corpus cases that had only an `(events …)` observation and no primary
  result clause.
- [Real components, not a bespoke module model](./2026-07-03-real-components-not-a-bespoke-module-model.md)
  — why the bootstrap uses real WebAssembly components (`wit-bindgen` core module → `wasm-tools
  component new`) rather than a hand-managed `wasm32-unknown-unknown` core module with an AST slot and
  trimmed imports; the WIT world makes "imports mirror the manifest" hold natively, which reverted the
  short-lived 0.3.0 import amendment. Includes the offline de-risk-spike findings.
- [The seed needs first-class functions](./2026-07-03-the-seed-needs-first-class-functions.md) — why the
  seed realizes functions and closures (core-semantics.md §Functions): the first Cadenza artifact is a
  compiler, which is not expressible without them.
- [Bootstrap targets the compiler directly](./2026-07-03-bootstrap-targets-the-compiler-directly.md) —
  why the staged path collapsed to seed interpreter → Cadenza compiler → self-hosting, dropping the
  re-author-the-interpreter-in-Cadenza rung; the reference interpreter stays the oracle, so IX/XIV hold.
- [The seed realizes a byte-sequence form so the Cadenza compiler emits component bytes](./2026-07-03-seed-realizes-bytes-so-the-compiler-emits-components.md)
  — why the codegen is authored in Cadenza (not the seed) and the seed realizes a `Bytes` value form:
  an attended halt when the build was about to write the codegen in Rust, and the seed↔compiler seam it
  hardened (bootstrap.md §"The Compiler Is Authored In Cadenza, Not In The Seed").
- [The Cadenza compiler emits the whole component](./2026-07-03-the-compiler-emits-the-whole-component.md)
  — why the compiler emits the complete component binary as a value rather than a core module a tool
  completes, so a derivation's bytes are a function of the Cadenza compiler alone and self-hosting is a
  clean fixpoint (no external wrapping tool in the byte path).
- [One accessor, everything is a record](./2026-07-03-one-accessor-modules-are-records.md) — why `.` is
  the sole record accessor (`a.b` is sugar for `(. a b)`), why modules/records/prelude namespaces are
  all records while maps stay dynamic, and the `(meta …)` metadata channel; killed a lowercase/uppercase
  dotted-atom heuristic that re-parsed meaning from an atom's spelling.
- [Exhaustion is a trap across the compiled seam](./2026-07-03-exhaustion-is-a-trap-across-the-compiled-seam.md)
  — why a derived component that exhausts the resource measure halts as a trap and is judged as agreeing
  with the interpreter's `exhausted` terminal condition; surfaced by the differential gate before growing
  the compiler to recursion, so the two recursion cases don't flip to a false `disagree`.
- [Decline, do not miscompile](./2026-07-03-decline-do-not-miscompile.md) — why a compiler grown
  incrementally MUST trap/decline a construct it cannot yet compile rather than emit divergent bytes or
  silently skip it, keeping "cannot yet" and "does wrong" observably distinct so a green differential gate
  means every compiled program agrees.
- [The corpus is a differential gate](./2026-07-03-the-corpus-is-a-differential-gate.md) — why the
  generated path is exercised against the oracle over every corpus case the compiler compiles, turning the
  executable-semantics corpus into a live regression surface as the compiler grows (agree/todo/skip/disagree).
- [The assembler lives in Cadenza](./2026-07-03-the-assembler-lives-in-cadenza.md) — why even the
  instruction-to-bytes assembly step is authored in Cadenza (a WAT-like structured layer folded to bytes),
  not delegated to a host `wat`-crate pass, so no part of the translation escapes the Cadenza compiler.
- [The compile seam is statically typed](./2026-07-03-the-compile-seam-is-statically-typed.md) — why the
  seed invokes the Cadenza compiler through a byte-to-byte interface (`compile : list<u8> -> list<u8>`)
  rather than through its dynamic value type, so no dynamic-language assumption is baked into the
  compiler's contract and a later generation can type-check the same seam; surfaced when the self-hosting
  harness needed the interpreted and compiled compilers to share one static type to be comparable.
- [Author Cadenza as static even though the seed is dynamic](./2026-07-03-author-cadenza-as-static-even-though-the-seed-is-dynamic.md)
  — why every line of Cadenza source is written as a well-typed static program (sum types + `match`, not
  runtime `Ast.is-*` kind-reflection) even though the seed defers type-checking, so the source is accepted
  unchanged by the later type-checking generation rather than rewritten and the §VII deferral stays a stage.
- [Uniform single-arity constructors eliminate cascading special cases](./2026-07-03-uniform-single-arity-constructors.md)
  — why all sum type constructors are single-arity functions (including "nullary" variants that take Unit),
  rather than nullary-as-pre-applied-Sums vs unary-as-Constructors, eliminating arity-based special cases in
  pattern matching, type synthesis, and compilation; the dual representation compounded (each feature checked
  "which kind?"), and adding unit broke all tests when one check was missed.
- [Types first-class in the dynamic seed sets up static self-hosting](./2026-07-03-types-first-class-in-dynamic-seed.md)
  — why the seed makes types first-class values even though it's dynamically checked (§VII defers checking,
  not types themselves), and why the AST is quotable as a sum type: compiling dynamically-written code to
  static is incredibly hard, but runtime-checked types written with type annotations transition smoothly to
  compile-time checking (move validation earlier, not infer what wasn't written); quote/unquote lets the
  compiler operate on AST values natively rather than string-tagged reflection.
