# Host functions are un-named: the language binds any WIT-typed host function, the vocabulary is the target's

*2026-07-05*

**What happened.** The language spec had baked in **four concrete host operations** — read-projection,
emit-event, read-blob, invoke-tool — naming them normatively in the frozen `host-interface-binding.md`
and anchoring the effect vocabulary to them. Those four are the **Hivemind target's** operation set,
not the Cadenza language. They were removed from the language: the sole language requirement is now
that a program can **bind to WIT-typed host functions** it declares, and *which* functions exist is
entirely the target's concern. An imported host function carries its **complete WIT signature**
(parameters, result, error) so the compiler can emit the import into the component's world from the
program's source alone. The manifest is the projection of the escaping effect row over whatever host
functions a program imports; purity is the empty row. The observable-behavior model generalized from an
`(events …)` clause (emit-only) to a `(host-calls …)` trace over arbitrary imported functions, with a
`(host-responses …)` fixture for functions that return values.

**Why.** Naming four concrete operations in a normative contract was a target leak — a concrete
technology choice living in a requirement rather than at the declared-default location (constitution
XIII). It also over-constrained the language to one target's world, when Cadenza is meant to be the
source-and-derivation tool for a *class* of capability-gated component systems. Un-naming is strictly
more general and strictly more honest: the language fixes only the *mechanism* (an import is a
WIT-typed host function the manifest enumerates), and each target's concrete world is pinned once at the
shared declared default, cited by both the Cadenza and target trees so the seam is one definition, not
two that drift ([[2026-07-04-cadenza-and-the-target-share-one-seam]]). The compiler itself is the
limiting case: it reaches **no** host function, so its import world is empty — it is not a special case
of the capability model but an instance of purity = the empty row.

**The requirement it drove.** `spec/contracts/host-interface-binding.md` advances to version 2:
§"Core Host Operations" (the four named bindings) is replaced by §"Imports Are WIT-Typed Host
Functions", §"Which Host Functions Exist Is The Target's Concern", and §"The Manifest Is A Projection
Of The Escaping Effect Row", naming no operation. `spec/capabilities/core-semantics.md` generalizes its
observable-behavior and effect-only-expression sections from events/emit to host calls.
`spec/contracts/build-tool-interface.md` gains §"The Compiler Imports No Host Function". The concrete
`hivemind-host` world becomes illustrative-only at `options/execution-model/wasm-component-model.md`.
The glossary's "Event" entry becomes "Host call". The corpus DSL replaces `(events …)` with
`(host-calls …)` plus a `(host-responses …)` fixture (spec/semantics/README.md). Supersedes the
"anchored to four ops" half of [[2026-07-04-the-host-interface-is-the-effect-vocabulary]] (revised in
place) and composes with [[2026-07-05-host-calls-suspend-as-replay-from-the-hosts-log]].
