# Cadenza and its target share one seam: the derivation tool and host interface are one definition, not two

*2026-07-04 (touchpoint #2 revised 2026-07-05)*

**What happened.** Cadenza is not a language in a vacuum — it is **the source language and derivation
tool for a specific target system** (a self-organizing agent pool over a durable event log, where
*behavior is data*: modules are published as source + capability manifest and run as sandboxed,
content-addressed components). That target has its own spec tree with the same architecture, the same
gate, and the same governance model as Cadenza. The two trees meet at a **seam**, and the seam must be
**one definition shared**, not two parallel definitions that agree today and drift tomorrow. This
learning records the seam explicitly so the correspondence is maintained deliberately.

**The seam has three frozen touchpoints, and they already correspond almost exactly:**

1. **Derivation.** The target's derivation contract requires: source → content-addressed component;
   reproducible (same source + pinned toolchain → byte-identical); performed by a *replaceable,
   capability-gated build tool* that is **not** in the frozen root; re-derivable and verifiable by any
   participant against the component hash bound in the module declaration. Cadenza's
   `build-tool-interface.md` is a near-mirror: canonical source tree in; component + manifest +
   machine-readable diagnostics out; the tool is itself a verified, reproducibly-derived component and
   is *not* part of any load-verify-run root; a new source language is a new tool. **These are the same
   object** — Cadenza's build-tool interface is the concrete realization of the target's derivation
   contract.

2. **Host interface.** The target's runtime-host-interface is a versioned, WIT-shaped world of host
   functions, pure by construction (no clock, randomness, ambient IO, or shared-memory concurrency),
   named by each module. Cadenza's `host-interface-binding.md` fixes the *mechanism* — an import is a
   WIT-typed host function the manifest enumerates, imports mirror the manifest exactly, and imports
   travel with the interface version — and names **no** operation. **The manifest is the escaping effect
   row** ([[2026-07-04-the-host-interface-is-the-effect-vocabulary]]): the language defines the seam's
   *shape*, and the target's world defines the *functions*, pinned once at the shared declared default.
   (Touchpoint revised 2026-07-05: version 1 named four fixed operations — read-projection, emit-event,
   read-blob, invoke-tool — which were a target leak; they are now the illustrative `hivemind-host` world
   in `options/execution-model/`, not language vocabulary.)

3. **The manifest / behavior-is-data shape.** The target's module-declaration contract requires a
   declaration carry source (not a binary), a source hash, the expected component hash, a capability
   manifest enumerating every projection read / kind emitted / host op called / tool invoked, a role,
   and a targeted interface version. Cadenza produces exactly that manifest and binds the component to
   exactly that hash. The manifest Cadenza emits **is** the manifest the target's declaration carries.

**Why this must be stated (not left implicit).**
- **Drift between two trees is the failure mode both were built to prevent.** Cadenza exists because a
  language's meaning had been scattered across implementations that drifted
  ([[2026-07-02-parallel-semantics-drifted]]); a *second* definition of the derivation interface or the
  host world — one in each tree — reintroduces exactly that hazard at the seam. The host-interface
  mechanism, the reproducibility guarantee, and the manifest shape must have **one** authoritative
  definition that both trees cite, and the target's concrete world must be pinned once at the shared
  declared-default location (`options/execution-model/`), cited by both trees rather than re-declared in
  each.
- **The governance floors must be consistent across the seam.** The target's "frozen root changes only
  by coordinated act / strands no log data without a migration path" and Cadenza's "component ABI
  changes only by coordinated act" govern the *same bytes* from two sides. An ABI change in Cadenza that
  the target's frozen-root discipline would forbid, or vice versa, is a latent inconsistency. They must
  be evaluated together.
- **Self-hosting closes across the seam, not just within Cadenza.** Cadenza's flywheel (the compiler
  rebuilds the compiler) is a *sub-loop* of the target's flywheel (the running system publishes and
  activates the modules that build its next generation). Cadenza's derivation tool is itself a module in
  the target. So "reproducibly derivable, content-addressed, capability-gated" must hold for the
  compiler-as-module under the target's rules, not only under Cadenza's.

**Consequences to hold.**
- **The host-interface mechanism is defined once in Cadenza; the target's world is defined once,
  shared.** Neither tree invents its own mechanism or its own copy of the world; both cite the pin at
  `options/execution-model/`.
- **A change to the derivation interface, the target's host world, or the manifest shape is a
  coordinated act in BOTH trees.** It is subject to Cadenza's ABI governance floor and the target's
  frozen-root discipline simultaneously.
- **This learning is Cadenza-side and descriptive of the seam;** the reciprocal note belongs in the
  target's tree. Capturing it here first records the coherence obligation from the side being actively
  specified.

**The requirements it drives.** `spec/contracts/build-tool-interface.md` and
`spec/contracts/host-interface-binding.md` are annotated (additively) that they realize the target's
derivation and runtime-host-interface contracts respectively, and that the concrete operation set and
reproducibility pin are the shared declared-default at `options/execution-model/` — one definition cited
by both trees. `spec/traceability.md` gains the seam mapping. No normative sentence is weakened; this
names an existing correspondence and binds the two governance disciplines together at the seam. Composes
with [[2026-07-04-the-host-interface-is-the-effect-vocabulary]] and
[[2026-07-04-fold-modules-are-provably-pure]].
