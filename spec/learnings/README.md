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
