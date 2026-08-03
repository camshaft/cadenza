# PR #1215 review comments — cdz-kernel/src/effect.rs + wasm_host.rs (v-agent-harness)

Mirrored from https://github.com/camshaft/cadenza/pull/1215 (PR: "cand: v-agent-harness — 1c88f1099").

## 1. ⚠ New required `EffectRequest.timeliness` field breaks cdz-agent-host build (Copilot, effect.rs:59) — CORRECTNESS / build-break
> `EffectRequest` now requires a `timeliness` field, but `cdz-agent-host` still constructs
> `EffectRequest` without it (e.g. `cdz-agent-host/tests/agent_runs_e2e.rs:38-42` and
> `cdz-agent-host/src/clock.rs:61-66`). This will fail to compile for the agent-host workspace/CI
> job; please update those call sites (or provide a constructor/helper that supplies the default).

This is the important one: adding a required field to `EffectRequest` in cdz-kernel leaves the
downstream `cdz-agent-host` crate's construction sites (clock.rs, agent_runs_e2e.rs) missing the
field → compile failure for that workspace. Since agent-host is a SEPARATE workspace, the main fleet
gate may not catch it. Either update those call sites or add a constructor/`Default` that supplies
the field. Worth confirming the agent-host CI job is green after this lands.

## 2. `timeliness` doc claims durable-log + executor routing, but neither is wired (Copilot, effect.rs:58, also :74) — doc/behavior
> The `EffectRequest::timeliness` doc comment says the field is on the durable log and that the
> executor reads it to select on-demand vs batch routing, but the kernel's durable
> `EventBody::Dispatched` frame doesn't record it (only kind/target/idempotency/etc.) and there are
> no current reads of `req.timeliness` outside tests. Please adjust this comment to reflect the
> current behavior (or wire timeliness through the durable dispatch frame and executor routing).

## 3. Forward-looking "doesn't yet … follow-up" comment goes stale (Copilot, wasm_host.rs:634) — doc
> This comment is written as a status update ("doesn't yet… follow-up"), which tends to become
> stale. Prefer a present-tense statement of the current behavior (guest effects default to
> Interactive because timeliness isn't part of the WIT surface).

Points 2+3 are the "don't document a field as doing more than it does / don't bake a timeline into
comments" pair — state current behavior (timeliness is carried but not yet on the durable frame or
read by routing; guest effects default to Interactive since it's not in the WIT surface).
