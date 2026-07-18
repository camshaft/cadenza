# pr559 — xtask emit_agrees_with_interp: emit-side BadArtifact silently counts as AGREE

Mirrored from GitHub PR #559 review comment (Copilot), id 3607418295.
PR: https://github.com/camshaft/cadenza/pull/559 (5-MR publish batch)
Location: `xtask/src/main.rs:3320` (fn `emit_agrees_with_interp`, body at 3361 on trunk)

## Reviewer comment (verbatim)
> `emit_agrees_with_interp` treats *any* emitted outcome as agreeing with an interpreter trap
> (`(_, Ran::Trap(_)) => true`), which includes `Ran::BadArtifact` (non-zero exit / hang /
> unrecognized verdict). That can silently count emit-harness failures as "agree", masking real
> emit pipeline breakage. Treat `BadArtifact` as coverage-not-yet (or as a disagreement) instead
> of agreement.

## VERIFIED real (git show trunk)
`fn emit_agrees_with_interp` (main.rs:3361) match:
```
(Ran::Value(a,_), Ran::Value(b,_)) => a == b,
(_, Ran::Value(_,_)) => false,
(Ran::Value(_,_), Ran::Trap(_)) => false,
(_, Ran::Trap(_)) => true,     // <-- catches (Ran::BadArtifact(_), Ran::Trap(_)) => true
_ => true,
```
The caller (main.rs ~3330) filters interp-side `Ran::Declined`/`Ran::BadArtifact` to `EmitOutcome::NotYet`
BEFORE comparing, but does NOT filter the EMIT side. So when the interpreter TRAPS and the emit path
returns `Ran::BadArtifact` (spawn fail / timeout-hang / unrecognized verdict — see the BadArtifact
constructors at 844/849/856/857/862/883), the `(_, Ran::Trap(_)) => true` arm counts it as AGREE.
=> an emit-harness failure on a trapping program is silently green, masking emit-pipeline breakage in
the differential gate. Copilot (accurate track record; this session's parser.rs:2099 was its only miss).

## Fix (per reviewer)
Add an explicit arm so an emit-side `Ran::BadArtifact` is NOT agreement — either coverage-not-yet
(NotYet, consistent with how interp-side BadArtifact is treated) or a Disagree. Likely also worth the
same guard on `ml_agrees_with_oracle` if it has the parallel shape.

## Owner
`xtask/src/main.rs` differential/emit gate harness — same ambiguity as PR#548 (xtask helper vs the
feature it gates). Filing to PM to route (v-fleet-tooling owns xtask; the concern is emit-vs-interp
differential coverage).

---
ROUTED to v-fleet-tooling (owns xtask), corpus-bugfix 2026-07-18, VERIFIED via grep. main.rs:3369
(_, Ran::Trap(_)) => true — wildcard on the EMIT side swallows Ran::BadArtifact (run-emitted
spawn-fail/timeout/unrecognized-verdict), so an emit-harness failure on a trapping program is silently
counted AGREE -> emit-pipeline breakage goes green in the differential gate. FIX: explicit
(Ran::BadArtifact(_), _) => NotYet/false arm BEFORE the (_, Ran::Trap(_)) catch-all; check ml_agrees_with_oracle
twin. amazon-q's socket flags + a parser.rs Copilot flag on the same PR were hallucinations (liaison dismissed).
