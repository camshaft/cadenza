# A host call suspends and resumes by replay from the host's log; the program holds no resume state

*2026-07-05*

**What happened.** Durable execution — the target's suspend-record-resume-anywhere agent step, where a
tool call is logged and shipped to another federated host to run asynchronously — was made a
first-class, mandatory property of the boundary rather than something a program author codes by hand.
Every imported host call is a **suspension point**, and resumption is Temporal-style **replay from a
log the host owns**:

- The program is invoked identically every time — `run(input)` — and holds **no** resume state.
- The host owns the ordered log of responses to the host calls made so far. At each host call the host
  either returns the logged response (**replay**; the real side effect is not re-performed) or, at the
  frontier, records the pending call and **suspends**, yielding `(fn, args)` as an opaque token up to
  the initial callsite.
- The host resolves that call however it likes — synchronously and locally, or async / federated /
  later / on another machine — appends the response to its log, and re-invokes `run(input)` from the
  start. Determinism fast-forwards the replay to one call further.
- The continuation is therefore exactly **(content-addressed component + input + response log)** — all
  canonical data, no serialized linear memory — which is why a suspended run resumes on **any**
  federated host.

Replay is the *semantics*; the *resumption mechanism* is the host's per-call choice, and determinism
makes every faithful choice byte-identical: (1) answer in-process with no suspend, (2) checkpoint and
resume the live instance in place (no teardown, memory stays hot), or (3) checkpoint and tear down,
resuming later by replay. The soundness tie: the response the host feeds in any mode must equal the
value it records in the log.

**Why.** Two properties Cadenza already has make this nearly free, and one target need makes it
mandatory. A run's observable behavior is a deterministic function of its input and its capability
responses (constitution III), so a faithful replay reconstructs the exact execution up to the frontier
without serializing a live stack — the log *is* the continuation. And host imports are the escaping
effect row (host-interface-binding.md), so "a boundary effect is a suspension point" unifies durable
suspend/resume with capability-safety: the same mechanism that makes authority legible makes execution
resumable. The target's federated, asynchronous tool calls cannot block a live wasm instance for
minutes and migrate it between machines; a canonical-data continuation can. In-process await (keeping
the instance live) was considered and is retained only as an optimization the host *may* choose, never
as the portable model, because a live-memory continuation cannot migrate.

**The requirement it drove.** `spec/capabilities/capabilities-and-effects.md` gains §"Every Host Call
Is A Suspension Point", §"Suspension Is Replay From The Host's Log", and §"A Durable Continuation Is
Canonical Data", making the escaping effect row and the suspend-replay boundary part of the mandatory
floor (retiring the former opt-in-only framing). The frozen `spec/contracts/component-abi.md` gains, as
a coordinated version-2 change, §"The Entry May Suspend On A Host Call": the entry's result
distinguishes normal completion, a trap, and a suspension carrying the pending host call, and the host
resumes by re-invoking with the same input. The concrete suspend/resume mechanism (host-owned
log/index/pending in the runtime store; the guest holds no host reference) is pinned at
`options/effects-model/algebraic-one-shot.md`. The corpus `(host-responses …)` fixture
(spec/semantics/README.md) *is* this replay log. Composes with
[[2026-07-04-durable-execution-is-effects-plus-determinism]] and
[[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]].
