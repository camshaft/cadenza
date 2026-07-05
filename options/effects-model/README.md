# Decision — Effects Model

**The decision.** The concrete shape of Cadenza's effect system: how a host import suspends and
resumes at the boundary, how an intra-program effect is raised and handled, and how the effect row is
tracked in the type system. The constitution and `capabilities-and-effects.md` require that a program's
escaping effects equal its manifest and that reaching an undeclared capability is a compile-time error,
but they do not fix the operational mechanism, which is what this decision pins.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- A program's escaping effect row equals its imported host functions; purity is the empty row
  (host-interface-binding.md §The Manifest Is A Projection Of The Escaping Effect Row).
- Reaching a host function the manifest does not enumerate is a compile-time rejection
  (capabilities-and-effects.md §Undeclared Capability Is A Compile-Time Error).
- A host call is a suspension point: the program yields to the host and resumes by deterministic
  replay, holding no resume state itself (capabilities-and-effects.md §Suspension Is Replay From The
  Host's Log; see `spec/learnings/2026-07-05-host-calls-suspend-as-replay.md`).
- Determinism and bounded termination are never downgraded (constitution III, V; Governance Floors).

## Choices

- [`algebraic-one-shot`](./algebraic-one-shot.md) — every host import is a suspending boundary effect
  resumed by host-owned replay; intra-program effects are algebraic operations discharged by lexically
  scoped handlers with one-shot (affine) continuations; the effect row is row-polymorphic and
  monomorphized to a closed set before the boundary. **The default.**

DEFAULT: algebraic-one-shot
