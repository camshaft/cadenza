# Component output never materialized

*2026-07-02*

**What happened.** Earlier Cadenza's design documents described WebAssembly-component output in
detail — a component model target, a WIT interface, artifact kinds, a nine-phase pipeline ending in
emission. A partial IR-to-WebAssembly backend existed and produced textual-format snapshots for small
fragments. But four generations in, no Cadenza program was ever actually compiled to a running
component: the backend was never wired to a whole-program compile, and the component target remained
a moving goal that each new compiler core chased and never reached.

**Why.** The byte-level target was never pinned first. The mapping from Cadenza types to the
component boundary, the calling convention, the canonical value form, and the reproducibility rules
were described in prose that shifted with each core, so there was no fixed thing to build *toward* —
only a design that moved as fast as the implementation. A compiler cannot converge on a target that
is redefined every time the compiler is.

**The requirement it drove.** The frozen contracts, written *before* the capabilities that depend on
them: [component-abi.md](../contracts/component-abi.md),
[deterministic-value-form.md](../contracts/deterministic-value-form.md),
[host-interface-binding.md](../contracts/host-interface-binding.md),
[determinism-and-fuel.md](../contracts/determinism-and-fuel.md), and
[reproducible-derivation.md](../contracts/reproducible-derivation.md). The authoring rule that
follows is recorded in [AGENTS.md](../../AGENTS.md): freeze the component ABI and the determinism
forms before writing the capabilities, because a capability references the ABI, and the reason this
reboot exists is that the byte-level target was never pinned first.
