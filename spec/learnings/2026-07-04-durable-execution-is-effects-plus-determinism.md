# Durable execution falls out of effects + determinism: a boundary effect is a persistable suspension point

*2026-07-04*

**What happened.** The target's **agent-step** model demands something stronger than in-memory one-shot
continuations: a step reads a view, runs one turn, then either emits an outcome **or suspends recording
a continuation of what it awaits** — and while suspended it **leaves no compute running**, is
**resumable on any participant**, **resumes exactly once** when its awaited results arrive, and
**rehydrates from the log rather than from memory**. The whole life of a unit of work is
reconstructable from the log alone. That is **durable execution** (workflow-as-code, à la Temporal):
suspend at an effect, persist what is awaited, resume later — possibly elsewhere — deterministically.
Cadenza must support it, and it **falls out of two commitments already made** rather than needing a new
mechanism.

**Why it falls out.** A boundary effect (an `await`, a tool call, a reasoning turn) is an **algebraic
effect operation** ([[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]): control
transfers to the handler, which may resume the captured continuation. Make that handler the **host
boundary**, and "the handler resumes the continuation" becomes "the runtime resumes the step when the
awaited result arrives." Two properties already required make the *durable* part exact:
- **Determinism** — a run is a pure function of its inputs and its recorded effect responses
  ([[2026-07-04-deterministic-replay-is-the-debugger]]). So resumption need not serialize a live stack:
  the step can be **deterministically replayed** from its transcript (the recorded effect responses on
  the log) until it reaches the suspension point, then continue with the newly-arrived result. This is
  the *same* mechanism as the replay debugger — recorded effect responses + determinism = exact
  reconstruction — applied to *resumption* instead of *observation*.
- **One-shot continuations** — the continuation is affine, resumed **at most once**
  ([[2026-07-04-linearity-is-surgical-not-core]]), which is exactly "resume fires exactly once." A
  multi-shot continuation would re-run awaited work an unbounded number of times and could not map to a
  single log-recorded resumption.

**The two realizations, and the language demand under both.** Resumption is either (a) **replay** —
re-run the deterministic step feeding recorded effect responses (lighter; matches the target's
"rehydrate the transcript" language; no stack to serialize), or (b) **serialized continuation** — the
captured continuation is written as data and reloaded. Cadenza's determinism makes **(a) the natural
default**. But *either* way, the language-level demand is the same and new:
- **What a durable continuation captures must be reconstructable as data.** A continuation that survives
  to the log cannot close over an unserializable host handle; it may close over **immutable values with
  a canonical byte form** ([[2026-07-04-immutable-heap-is-acyclic-so-reference-counting-is-complete]])
  and **capability references named in the manifest**, both of which reconstruct deterministically. This
  constrains what may cross a suspending effect — a real typing obligation, not a runtime hope.
- **A step is a deterministic function of (view, transcript).** Its only nondeterminism is the recorded
  effect responses — including the reasoning turn, which is *itself* a capability-gated effect whose
  result is recorded, never in-module nondeterminism. So "the agent is nondeterministic" and "the step
  replays deterministically" are both true: the nondeterminism is quarantined into recorded effect
  responses ([[2026-07-04-the-host-interface-is-the-effect-vocabulary]]).

**Prior art.** **Temporal** / durable-execution engines (replay workflows from an event history — the
closest match to the target's transcript model); **algebraic-effect** continuations (Koka, OCaml 5,
Unison abilities); Unison's storing computations as content-addressed data is the nearest existing
system to "a continuation is data on a log."

**Consequences to hold.**
- **Suspension is at effect boundaries only.** A step suspends where it performs a boundary effect
  (await/tool-call), not at arbitrary points — the effect row marks exactly where a step may become
  durable, so durability is legible from the type.
- **The debug replay and the resume replay are one mechanism.** Both reconstruct a run from recorded
  effect responses; a generation that realizes one has most of the other.
- **Determinism of resumption is mandatory, not incidental.** If a step's replay diverged from its
  original run (nondeterminism the effect system did not capture), resumption would be unsound — so this
  is another reason nondeterminism must be *only* through recorded effects (Constitution III).

**The requirements it drives.** `spec/capabilities/capabilities-and-effects.md` gains a §"A Boundary
Effect Is A Durable Suspension Point": a computation MAY suspend at a boundary effect; what a suspended
continuation captures MUST be reconstructable as canonical-form data plus manifest capability
references; resumption MUST be deterministic (by replay of recorded effect responses or by a serialized
continuation), and a continuation MUST resume at most once. Composes with
[[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]],
[[2026-07-04-deterministic-replay-is-the-debugger]], and
[[2026-07-04-the-host-interface-is-the-effect-vocabulary]].
