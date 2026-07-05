# The host interface IS the effect vocabulary: the manifest is the escaping effect row over a target's WIT-typed host functions

*2026-07-04 (revised 2026-07-05)*

**What happened.** The abstract "capabilities / effects" of the type system are anchored to a
**concrete but target-owned operation set**: the manifest a program declares *is* the effect row that
escapes to the host, and the labels in that row are exactly the **WIT-typed host functions the program
imports**. The effects-as-capabilities model
([[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]) stops being abstract without
Cadenza having to fix *which* functions exist.

> **Revision (2026-07-05).** This learning first anchored the vocabulary to **four fixed operations**
> (read-projection, emit-event, read-blob, invoke-tool) taken from `host-interface-binding.md`'s then
> §"Core Host Operations". Those four are a concrete choice of **one target** (Hivemind), not part of
> the Cadenza language — baking them into a normative contract was a target leak (Constitution XIII:
> a concrete technology choice belongs at the declared-default location, not in a normative
> requirement). The contract is now version 2: §"Imports Are WIT-Typed Host Functions" fixes only the
> *mechanism* — an import is any WIT-typed host function the manifest enumerates — and names no
> operation. The durable insight below is unchanged; only the "fixed to four ops" anchoring is removed.

**Why — the target pins the *world*, not the language.** Cadenza is the source language and derivation
tool for systems where **behavior is data**: units of behavior are published as source plus a
capability manifest, and run as sandboxed, content-addressed components against a **pure-by-construction
host interface** the *target* defines. That target's runtime interface is a versioned WIT-shaped world
offering some set of host functions and *no ambient nondeterminism* — no clock, no randomness, no
ambient IO, no shared-memory concurrency. Cadenza mirrors this exactly: a program imports the WIT-typed
host functions its manifest enumerates, and "imports mirror the manifest" makes the escaping row equal
the imports. The effect vocabulary is therefore **fixed per target by the world a component names**,
not a free design space and not fixed by the language.

**What this makes concrete (previously abstract).**
- **The manifest is the escaping effect row.** "Imports mirror the manifest exactly"
  (`host-interface-binding.md`) is, under the effects model, "the component's escaping effect row equals
  its imports." Importing a host function `f : A -> B` is the effect `f`; the row *is* the manifest.
- **Reads and emits are effects, not pure returns.** A host function that returns a value the program
  uses (a projection read, a blob read, a tool invocation) is a capability effect — its response is part
  of what a run depends on, so it is what a deterministic replay records
  ([[2026-07-04-deterministic-replay-is-the-debugger]]). A function returning unit (an emit) is the
  degenerate case.
- **Imports are strongly WIT-typed.** A host import declares its complete signature (parameters,
  result, error), sufficient for the compiler to emit it into the component's world — so an effect-row
  label carries a type, not just a name.
- **Attenuation is scoped over these labels.** A handler forwarding a narrower row
  ([[2026-07-04-capabilities-attenuate-a-handler-forwards-a-narrower-row]]) forwards a subset of the
  host functions it holds — e.g. grant a callee one imported function but not another.
- **Purity is the empty row.** A program that imports no host function is deterministic and legibly so
  from an empty manifest — the exact obligation a *fold* role carries in the target
  ([[2026-07-04-fold-modules-are-provably-pure]]).
- **The compile-time tier's purity is the same empty row.** Macro expansion / generic reduction reach
  no host function ([[2026-07-04-compile-time-evaluation-is-one-tier]]) — compile-time evaluation is
  pure *because* its effect row is empty. So too is the **compiler itself**, a component with an empty
  import world.

**The seam to hold (drift is the enemy).** Cadenza and its target are two spec trees with the same
architecture and gate. What must stay **one definition**, not two that drift
([[2026-07-02-parallel-semantics-drifted]]), is the *mechanism* — a WIT-shaped world, imports mirror
the manifest, imports are strongly typed — plus, for a given target, the concrete world that target
freezes. The concrete world lives at the declared-default location (`options/execution-model/`, shown
there as the illustrative `hivemind-host`) and MUST be the same world the target's runtime freezes;
Cadenza's `host-interface-binding.md` names *no* function precisely so no target's vocabulary is baked
into the language. See [[2026-07-04-cadenza-and-the-target-share-one-seam]] for the cross-tree coherence.

**The requirements it drives.** `spec/contracts/host-interface-binding.md` §"Imports Are WIT-Typed Host
Functions" (version 2): an import is any WIT-typed host function the manifest enumerates; the escaping
effect row equals the imports; a component reaching no host function has an empty manifest.
`spec/capabilities/capabilities-and-effects.md` ties the effect row to the imported functions without
naming any. The concrete per-target world stays in `options/execution-model/` as the shared,
single-definition binding. Composes with
[[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]],
[[2026-07-04-fold-modules-are-provably-pure]], and
[[2026-07-04-capabilities-attenuate-a-handler-forwards-a-narrower-row]].
