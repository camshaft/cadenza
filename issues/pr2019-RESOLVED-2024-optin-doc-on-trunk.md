# PR #2019 review — cdz-agent-host/src/factory.rs + metrics.rs (v-agent-harness-host) — OPEN — doc-vs-impl [VERIFIED, LOW] (batched)

https://github.com/camshaft/cadenza/pull/2019 (per-effect metric surface — EffectMetrics + MeteredExecutor).
Copilot 2 inline, same class: the docs claim the daemon wraps executors with `MeteredExecutor`, but that
wiring isn't in the built-in registration path.

## `MeteredExecutor` doc says "the daemon wraps each real leaf executor" but `LiveExecutorSet::build` registers leaf executors DIRECTLY (no wrap) (Copilot, factory.rs:67,74,83,505 + metrics.rs:19) — doc-vs-impl [VERIFIED]
> This doc comment states that "The daemon wraps each real leaf executor" with `MeteredExecutor`, but
> `LiveExecutorSet::build` currently registers `ClockExecutor`/`HttpExecutor`/`ModelExecutor` directly (no
> wrapping). Either wire the decorator in (and surface the shared `Arc<EffectMetrics>` somewhere) or adjust
> the docs to say callers/daemons *may* wrap executors to record metrics.
> [metrics.rs:19] The module docs read as if per-effect metrics are automatically captured by the
> daemon/exporter, but this crate does not wire `MeteredExecutor` into the built-in executor registration
> path … describe `EffectMetrics` as opt-in counters recorded by wrapping executors, rather than implying
> it is always present.

VERIFIED in the #2019 diff: `MeteredExecutor` is defined (factory.rs:37) and its doc (factory.rs:35) says
"The daemon wraps each real leaf executor with one of these sharing ONE `Arc<EffectMetrics>`". But
`LiveExecutorSet::build` still registers `ClockExecutor`/`HttpExecutor`/`ModelExecutor` directly via
`with_effect` — the ONLY `MeteredExecutor::new` call sites are in the unit tests (diff :114-115). So no
daemon/built-in path wraps executors; `EffectMetrics` is currently opt-in-and-unwired. The docs (both
factory.rs and metrics.rs:19) overclaim automatic capture. LOW/doc-vs-impl. Fix per Copilot: either (a)
wire `MeteredExecutor` into `LiveExecutorSet::build` (wrap each leaf, share one `Arc<EffectMetrics>`,
surface it for the exporter), or (b) reword the docs to "callers/daemons MAY wrap executors with
`MeteredExecutor` to record per-effect metrics" — opt-in, not always-present. Given per-effect metrics look
staged like the rest of the observability surface (parse/define ahead of daemon wiring, cf. #1981/#2001),
the doc reword is the honest fix now; wiring is a follow-up slice. v-agent-harness-host owns
cdz-agent-host/src. (PR OPEN → foldable.)
