# Deterministic replay is the debugger: determinism buys time-travel debugging for free

*2026-07-04*

**What happened.** Runtime debugging is made **observation over deterministic replay**, not print-
statement instrumentation. Because a run is a pure function of its inputs and its capability responses
(Constitution III) and is bounded by the deterministic resource measure (Constitution V), a run is
**losslessly replayable**: re-running the same program with the same inputs and the same recorded
capability responses reproduces the identical run, step for step. So the agent debugs by **re-running
and observing** — inspecting the value of any expression, the environment at any point, the sequence of
effects, stepping forward (and, from a checkpoint, backward) by the resource measure — never by editing
the program to emit a value it wants to see.

**Why determinism hands this over for free.** The property adopted for *safety* — determinism — is
exactly the property production debuggers spend enormous effort to *manufacture*. Tools like `rr` and
Pernosco achieve reverse/time-travel debugging by recording and taming the nondeterminism of an ordinary
process (scheduling, syscalls, clocks). Cadenza has **no latent nondeterminism to tame**: every source
of it is a declared capability whose responses are legible and recordable (overview §4,
[[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]). Therefore:
- **Replay is exact with a tiny recording.** To reproduce a run, record only its inputs and its
  capability responses (the effect operations that crossed the boundary) — everything else is a pure
  function of those. A deterministic, capability-free program needs *nothing* recorded but its input.
- **"Step by the resource measure" is well-defined.** Execution is accounted against fuel (Constitution
  V), so there is a canonical, deterministic notion of "one step" and "the state at step N" to seek to —
  the debugger's timeline is the fuel axis, not wall-clock.
- **State is inspectable because values are immutable and canonical.** A value at any point has one
  canonical byte form ([[2026-07-04-immutable-heap-is-acyclic-so-reference-counting-is-complete]]), so
  the debugger can display any intermediate value unambiguously, and structural sharing means a snapshot
  is cheap.

**The boundary that must not be crossed: the debug view is a tool-time projection, NOT observable
behavior.** `core-semantics.md` §"Observable Behavior Is A Defined Projection Of A Run" fixes observable
behavior as exactly *terminal condition + normal-termination value + ordered events*, and explicitly
**excludes internal representation and timing**. The replay/observation view is richer than that — it
sees intermediate state and per-step structure — so it MUST be a **tool-time projection that another
program cannot depend on** and that is **not part of observable behavior**. Otherwise the debugger would
silently widen the semantics and two runs equal under observable behavior could be "different" to a
program that inspected their traces. Debugging observes the run; it does not redefine what the run *is*.

**Why this serves the zero-feedback agent.** "Never insert printing just to figure out a value" is the
runtime twin of "never instrument to learn a type" ([[2026-07-04-the-compiler-is-a-queryable-oracle]]):
static facts come from querying the compiler; runtime facts come from replaying and observing. Both keep
the agent's program *unmodified* while it investigates — instrumentation would change the very artifact
under study (and, being an edit, would need its own compile). The agent's loop is: run → observe the
replay → (if wrong) query/transform → re-run, all over strict executables and machine-readable
observations, with no human and no `print` in the loop.

**Prior art.** `rr` / Pernosco (record-and-replay + reverse debugging over tamed nondeterminism);
time-travel debuggers generally. The difference is that they *reconstruct* determinism at cost; Cadenza
*starts* deterministic, so replay is a consequence of the execution model rather than a subsystem bolted
onto it — the same "we already built the engine, this names it" pattern as
[[2026-07-04-compile-time-evaluation-is-one-tier]].

**The requirements it drives.** `spec/capabilities/tooling-and-lsp.md` (query/observe surface) gains a
§"Runtime Facts Come From Deterministic Replay": a run is reproducible from its inputs and recorded
capability responses; a debug session observes intermediate state, environment, and effect order over
that replay; and stepping is accountable against the resource measure. A requirement (in that section or
`core-semantics.md`) states that the debug/replay view is a **tool-time projection that is not part of
observable behavior and that no program may depend on**, preserving `core-semantics.md` §Observable
Behavior. `spec/contracts/determinism-and-fuel.md` is annotated that the determinism and fuel guarantees
are what make lossless replay and fuel-indexed stepping possible (a consequence, not a new obligation).
Composes with [[2026-07-04-the-compiler-is-a-queryable-oracle]] and
[[2026-07-04-program-transformation-is-a-program]] — the three faces of "a program is data the compiler
serves": transform it, query it, replay it.
