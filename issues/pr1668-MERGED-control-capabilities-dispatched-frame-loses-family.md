# PR #1668 review comments — cdz-kernel/src/{kernel,executor}.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1668 (MERGED — answer control/capabilities INLINE in the kernel).

## 1. Inline control/capabilities Dispatched frame drops content_type.family → crash-recovery misclassifies it as `emit` (Copilot, kernel.rs:508) — correctness/durability [VERIFIED]
> The inline `control/capabilities` path appends `EventBody::Dispatched { kind: req.kind.clone(),
> target: req.target.to_string(), … }`, but a `Dispatched` frame does NOT record
> `req.content_type.family` (only the kind atom via `kind.family()`). A crash between this durable
> dispatch and the subsequent `EffectResult` leaves an OPEN dispatch that recovery can't identify as
> `control/capabilities` — a re-driver would treat it as a real `emit` (control requests carry
> `EffectKind::Emit`) instead of re-answering capabilities inline, and a timeout path would deliver an
> unexpected `TimedOut` to a capabilities query.

VERIFIED against the diff: the inline path (kernel.rs, under `req.content_type.family == CAPABILITIES`)
appends `EventBody::Dispatched { id, kind: req.kind.clone(), target: req.target.to_string(),
idempotency_key, deadline_ms: None, token }` — NO family field. Since a control request's `kind` is
typically `EffectKind::Emit`, a recovered open dispatch is indistinguishable from a real emit → recovery
misclassifies it (re-drives as emit, or delivers a spurious TimedOut to the capabilities query). MED
(narrow crash window between Dispatched and EffectResult, but a genuine persistence-layer misclassification
on a load-bearing recovery path). Fix per Copilot: record the content-type family (+ version) in the
durable dispatch, OR add a distinct durable variant for kernel-answered control dispatches, so recovery
deterministically re-answers `control/capabilities`. Recommend v-agent-harness evaluate — this is the
kernel's durable event log, so correctness here matters for crash recovery.

## 2. "The real drive path is always a `CompositeExecutor`" doc overclaims (Copilot, executor.rs:40) — doc/accuracy
> The doc comment claims "The real drive path is always a `CompositeExecutor`", but the kernel APIs
> accept any `Executor` and there are in-repo callers/tests that pass leaf executors directly.

Reword to a conservative statement ("the production drive path uses a `CompositeExecutor`; the APIs accept
any `Executor`, and tests pass leaf executors directly") so it isn't contradicted by the leaf-executor
callers. LOW/doc.
