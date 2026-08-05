# PR #1959 review — cdz-agent-host/src/async_host.rs (v-agent-harness-host) — MERGED — concurrency [MECHANISM CORRECTED; residual latency point VALID-LOW]

https://github.com/camshaft/cadenza/pull/1959 (wire the admin control interface into the host loop).
Copilot (id 3709666446) raises an admin-vs-inbox starvation concern. The STATED MECHANISM is wrong (tokio
`select!` is not first-listed-order), but a milder latency point survives. Relaying with the correction so
the owner doesn't apply a fix that would BACKFIRE.

## `select!` inbox arm before admin arm → "no fairness, continuous inbox starves admin" (Copilot, async_host.rs:229) — MECHANISM INCORRECT, residual LOW
> The `tokio::select!` arm for inbound events (`rx.recv()`) is placed before the admin control arm. Since
> `select!` provides no strict fairness guarantee, a continuously-ready inbox can delay (or effectively
> starve) admin requests under load… Consider prioritizing admin requests (e.g., place the admin arm
> before the inbox arm…), or explicitly draining at least one pending admin request per loop iteration…

VERIFIED the code (inbox `rx.recv()` arm at :229 precedes the `admin_rx.recv()` arm; `tokio = "1"`, no
`biased;`). BUT the premise is wrong: **`tokio::select!` polls its branches in RANDOM order by default**
(that randomization IS its fairness mechanism) — it does NOT poll top-to-bottom. So arm ORDER in the
source is irrelevant; when both inbox and admin are ready, admin wins ~50% of the time. A continuously
-ready inbox therefore does NOT starve admin by ordering. Copilot's "place admin arm first" fix would do
nothing (order-independent), and adding `biased;` to force admin-first would actually CREATE inbox
starvation — the opposite hazard. So DON'T apply the literal suggestion.

The residual, real (LOW) point: the inbox arm runs `host.deliver(...).await` — a full session turn —
synchronously in the iteration, so admin-request LATENCY under sustained inbox load is bounded by
(turn duration) × (expected selections until admin wins), not unbounded starvation. If admin
responsiveness under heavy inbox load is a real requirement, the sound fix is NOT reordering but bounding
per-iteration inbox work (already one msg/iteration) or servicing a pending admin request before the next
deliver — but given random fairness + one-turn-per-iteration, this is LOW and likely fine as-is. Recommend:
correct the record (mechanism), and treat any change as an optional latency-hardening, not a
starvation-bug fix. v-agent-harness-host owns cdz-agent-host/src.

(Discipline note: this is a verify-the-mechanism case — the finding SOUNDS like a real concurrency bug,
but tokio's documented random-poll default falsifies the ordering premise; relaying it unquestioned would
have prompted a backfiring `biased;`/reorder fix.)
