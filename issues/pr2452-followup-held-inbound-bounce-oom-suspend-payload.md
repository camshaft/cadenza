# PR #2452 (MERGED) follow-up — cdz-agent-host async_host.rs + lifecycle.rs (v-agent-harness-host) — 3 Copilot findings [VERIFIED post-merge]

https://github.com/camshaft/cadenza/pull/2452 — MERGED (trunk `0e12d9bac`, branch cand/v-agent-harness-host-b7a58afe9,
lifecycle/suspend+resume executor + loop hold/replay gate I4). Copilot posted 3 inline AFTER merge → follow-up relays,
not pre-merge folds. All 3 verified against trunk source.

## c3 — suspend/resume silently DROP caller payload; inconsistent with terminate's strict handling (Copilot, lifecycle.rs:110) — provenance/consistency [VERIFIED, LOW-MED]
> suspend/resume are documented as taking no payload, but perform() currently accepts and ignores any payload for
> lifecycle/suspend and lifecycle/resume (including blob refs). That silently drops caller-provided bytes and is
> inconsistent with terminate's strict payload handling.
VERIFIED (lifecycle.rs:126-131): the `is_terminate` arm matches `req.payload` exhaustively — Inline→lossy-decode,
None→empty, Blob→PERMANENT error (the "silent-drop hides a bug" rationale you wrote). But `is_suspend` → `Suspend{target,by}`
and the else → `Resume{target,by}` construct the op WITHOUT touching `req.payload` at all. So a caller who attaches a
payload (esp. a Blob ref) to suspend/resume gets it silently dropped — the exact inconsistency terminate's strict arm
was written to avoid. Cheapest durable fix: mirror terminate's guard on the suspend/resume path (reject non-None payload,
or Blob→permanent, and document None-only), so the strictness is uniform across the family.

## c1 — held-inbound replay drops None/FoldRefused even when a cross-session Emit set reply_to → silent Emit failure (Copilot, async_host.rs:519) — delivery-semantics [VERIFIED-STRUCTURE, design call]
> Held inbound replay currently drops deliveries that fail with None/FoldRefused, even when the original Inbound had
> reply_to set (cross-session Emit). That bypasses the delivery-failure bounce path used in the main inbox arm, so an
> Emit to a suspended session that later terminates can fail silently.
VERIFIED-STRUCTURE (async_host.rs:512-519): the replay arm matches `Some(Ok(()))|None|Some(Err(FoldRefused)) => {}` —
i.e. None/FoldRefused are swallowed, unlike the main inbox arm which bounces delivery failures. Your inline comment
states the design rationale: "a held inbound has no live emitter awaiting it." Copilot's counter is that a cross-session
**Emit** CAN carry reply_to, so a held-then-dropped Emit to a session that terminates while suspended fails silently
(no bounce to the emitter). Whether a held Inbound can actually carry a live reply_to (or whether the emit path already
resolved/detached it before holding) is YOUR semantics call — if held Inbounds never retain a live reply_to, the comment
is correct and this is a DECLINE-with-doc; if they can, the replay arm should route reply_to failures through the same
bounce path as the main arm. Coordinate w/ v-agent-harness (owns EffectKind::Emit + Inbound kernel side).

## c2 — held_inbound is an unbounded Vec fed from an unbounded inbox → OOM risk under sustained suspend (Copilot, async_host.rs:363) — resource/backpressure [VERIFIED-STRUCTURE, LOW-MED]
> held_inbound is an unbounded Vec fed from an unbounded inbox channel. If a session is suspended while producers
> continue sending to it, this can grow without bound and risk process OOM (the comment says "bounded in practice",
> but there is no enforcement/backpressure here).
VERIFIED-STRUCTURE (async_host.rs:363): `held_inbound: Vec<Inbound>` with the comment "A bounded buffer in practice
(only accumulates while a target is suspended)." Copilot's point: nothing ENFORCES the bound — a suspended session whose
producers keep sending grows the Vec without limit. Real resource concern (LOW-MED, needs an adversarial/misbehaving
producer + a long suspend). Options are yours: a documented cap + shed/bounce policy on overflow, or an explicit
"suspend is short-lived by contract, unbounded-hold is acceptable" doc that turns "in practice" into a stated invariant.

All 3 on a MERGED PR → follow-up work at your discretion; c3 (suspend/resume payload) is the cheapest and most clearly a
consistency defect; c1/c2 are deliberate-decision items (delivery-semantics + resource policy).
