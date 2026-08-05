# PR #2013 review — cdz-agent-host/src/metrics.rs (v-agent-harness-host) — MERGED — encapsulation + latent-concurrency + doc [VERIFIED] (batched)

https://github.com/camshaft/cadenza/pull/2013 (metric SURFACE — hermetic HostMetrics counters). Copilot 3
inline. Batched.

## `record_*` mutators are `pub` + `AgentHost::metrics()` returns `&HostMetrics` → any downstream crate can bump/corrupt host-owned counters (Copilot, metrics.rs:66 & :68) — encapsulation [VERIFIED, LOW-MED]
> `HostMetrics` mutation methods are `pub`, and `AgentHost::metrics()` exposes `&HostMetrics`, so
> downstream crates can arbitrarily bump counters and corrupt host-owned metrics. Make the `record_*`
> methods crate-private so only the host loop can mutate counters (snapshots can remain public).
VERIFIED: `record_turn` (metrics.rs:59), `record_delivery_to_unknown_session` (:68), and the other
`record_*` are `pub`; if `AgentHost::metrics()` hands out `&HostMetrics`, an external caller can invoke them
and desync the host's own counting. Fix: `pub(crate)` on the `record_*` methods (keep `snapshot()` + the
snapshot type `pub`). LOW-MED — needs a misbehaving in-repo consumer, but the surface shouldn't exist.

## `snapshot()` loads counters independently while `record_turn` bumps several atomics → the documented `turns_delivered == turns_ok + turns_err` invariant can be violated (Copilot, metrics.rs:87) — latent-concurrency [VERIFIED, LOW/latent]
> The code/doc comments claim `turns_delivered == turns_ok + turns_err`, but `snapshot()` loads each
> counter independently (and `record_turn()` updates multiple atomics). Under concurrent snapshots/export,
> this invariant can be violated … Consider deriving `turns_delivered` from `turns_ok + turns_err` inside
> `snapshot()` (or otherwise remove the equality claim).
VERIFIED: `record_turn` (metrics.rs:60-65) does `turns_delivered.fetch_add(1)` THEN `turns_ok`/`turns_err
.fetch_add(1)` — non-atomic across the pair; `snapshot()` (:76) `.load(Relaxed)`s each independently. A
snapshot BETWEEN the two `fetch_add`s reads `turns_delivered` bumped but ok/err not → invariant broken in
the returned snapshot. CALIBRATION: the module already documents single-threaded-loop-is-sole-writer +
"Not cross-counter atomic" (metrics.rs:22-23,75), and `HostMetrics` is NOT currently Arc-shared to another
thread — so the torn read is LATENT (only reachable once an exporter reads `snapshot()` from a DIFFERENT
thread than the loop). Still, the cheap fix future-proofs it: derive `turns_delivered = turns_ok +
turns_err` in `snapshot()` so the equality is ALWAYS true regardless of interleaving (and drop the
independent `turns_delivered` load). LOW/latent — do it now while the exporter is being built, before a
cross-thread reader lands.

## module doc has plan-narrative ("Operator directive", "Concierge-sequenced", follow-up-slice refs) (Copilot, metrics.rs:5) — doc-clarity [VERIFIED, LOW cosmetic]
> The module-level docs include process/plan narrative … rather than describing the module's current
> behavior and invariants … consider rewriting the intro to just state what the metric surface is and how
> it is intended to be consumed.
VERIFIED — LOW/cosmetic. Present-tense rewrite (what the surface IS + how the exporter/status consumes it).
Batchable. v-agent-harness-host owns cdz-agent-host/src.
