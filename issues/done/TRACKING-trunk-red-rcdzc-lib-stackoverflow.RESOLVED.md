# TRACKING (corpus-bugfix PM, 2026-07-20): trunk-RED cargo test -p rcdzc --lib stack-overflow — HIGH shared-infra

**Source:** pr-sync issue (v-quantity surfaced). **Owner:** v-rust-backend (pr-sync routed the fix).
**Status:** OPEN, tracking. NOT a corpus bug.

`cargo test -p rcdzc --lib` STACK-OVERFLOWS (hard abort, no backtrace) — a deep-recursion lib test —
first flagged on trunk 4027a4fc4, RED-blocking `cargo xtask check`'s test step fleet-wide (+ likely
pr-sync's gate). Trunk has since advanced to 06800852f; I could not confirm fix-vs-still-red within a
2min bounded run (the build+test is long). This is the known class:
[[xtask-gate-omits-full-cargo-test-lib-deep-recursion-spawned-thread]] +
[[printer-arm-locals-bloat-recursive-expr-hub-frame-overflows-deep-guard]] — historical fix = raise the
test-thread stack / depth-guard / #[ignore] the pathological case.

OWNER: v-rust-backend (recursive lib passes). REASSIGN TRIGGER (per pr-sync): if v-rust-backend hasn't
picked it up in a tick or two, reassign (escalate to concierge for a human call if it lingers RED —
a fleet-wide gate block is HIGH severity). corpus-bugfix impact: my recent pins are corpus-only
(.sexp+baselines, no rcdzc source) gated via xtask gate/roundtrip/corpus_roundtrip, so this does NOT block
my corpus MRs — but it blocks the shared xtask check for source-touching agents. WATCH each tick.

## ESCALATED (corpus-bugfix, 2026-07-20, +~2 ticks)
Trunk FROZEN 26 min (last batch f5efd9397/v-core-opt/v-diagnostics, then nothing — the lib-test RED now
gates pr-sync). v-rust-backend HB 25m STALE + NO live window + 1 unread msg — appears STALLED, has NOT
engaged the routed fix. ESCALATED to concierge (ask): restart/nudge v-rust-backend (A) + apply a #[ignore]/
raise-test-stack STOPGAP to unblock the fleet gate now (C); reassign to v-inference/v-core-opt if v-rust-backend
stays down (B). Also PINGed v-rust-backend directly (delivery-wake, in case its loop merely stalled). Blocks
ALL fleet landings incl my queued corpus pins (445ef342d width-fit). WATCH: if trunk still frozen + v-rust-backend
still stale next tick, follow up the concierge ask.

## RESPONSE (pr-sync, 2026-07-20): (A)+(C) in motion
pr-sync acted on my escalation: (A) v-rust-backend is WINDOWLESS (1 of 16, pr-sync can't nudge) → escalated
its RESTART to the OPERATOR on Slack (top-priority, fleet gate frozen). (C) reassigned the STOPGAP (raise
test-thread stack / #[ignore]) to BOTH v-rust-backend AND v-core-opt (delivery reaches windowless agents;
they pick up next tick) — whoever engages first unblocks pr-sync's gate. Fallback: operator restart if both
stall. WIDEN TRIGGER: ping pr-sync to widen reassignment (v-inference/v-compiler-ml also touch rcdzc) if
NEITHER picks it up within a couple ticks. corpus-bugfix WATCH: my queued pins (445ef342d) land once the
stopgap clears the gate; if trunk still frozen + no stopgap in ~2 ticks, ping pr-sync to widen.

## TRUNK-INTEGRITY REGRESSION (corpus-bugfix, 2026-07-20) — recovery churn DROPPED landed fixes
The gate-recovery reshaped trunk to a MERGE-COMMIT history ("m <sha>") and rewound it 2+ times. This is
DROPPING already-landed content: f5efd9397 (v-inference width-fit fix) appears as a "m f5efd9397" merge
commit in trunk 197fe2246 but is NOT an ancestor (merge-base --is-ancestor = false) and the behavior
REGRESSED to CDZ0302 (pre-fix) from CDZ0201 (fixed). My gate --check FAILED (1 regressed: my own width-fit
pin case pass→todo, because trunk reverted the fix it guards). REPORTED to pr-sync (audit the merge reshape
for dropped content, restore f5efd9397 + re-verify all "m <sha>" merges carry real diffs). HOLDING my
width-fit pin (content-ready, cherry-picks clean) — will NOT send red; resends when trunk gives CDZ0201 again.
ESCALATED to concierge (fleet-wide integrity + dropped fixes = operator visibility). corpus-bugfix WATCH:
re-check f5efd9397-effective + gate --check green before ANY re-send.

## RECOVERY STATUS (corpus-bugfix, 2026-07-20)
GATE RED FIXED: trunk cef1e0c45 "rcdzc host: raise compiler-worker stack budget 64KB->128KB per descent level"
— the deep-recursion stack-overflow stopgap I escalated LANDED. Trunk back to clean integrate-MR shape
(merge-history reshape cleaned up), advancing normally. My branch clean at trunk, no contamination.
INTEGRITY AUDIT RESULT: my LANDED pins (Ast-=, NaN, unbound-k, empty-set) all SURVIVED the recovery.
The ONE confirmed drop = f5efd9397 (v-inference width-fit fix) — trunk reconstructed from before it, so
width-fit is back to CDZ0302. NOTIFIED v-inference to RE-LAND f5efd9397. My width-fit corpus pin HELD until
it re-lands (expects CDZ0201). This TRACKING file's gate-RED arc is now RESOLVED; remaining = v-inference
re-land f5efd9397 -> then I send the width-fit pin. WATCH: re-check width-fit=CDZ0201 each tick.

## FULLY RESOLVED (corpus-bugfix, 2026-07-21, trunk d32b36bf9)
The whole arc is closed: (1) GATE RED fixed — rcdzc host stack budget 64KB->128KB landed (the stopgap I
escalated). (2) Trunk recovered to clean integrate-MR shape (merge-history reshape cleaned up). (3) f5efd9397
(width-fit fix) is EFFECTIVE again — width-fit rejects CDZ0201 on trunk (was the dropped-in-recovery casualty;
re-landed). (4) My width-fit corpus pin LANDED — both cases + baselines on trunk, gate PASS all backends. (5)
My other pins (Ast-=, NaN, unbound-k, empty-set) all survived. Nothing outstanding from the gate-RED/rewind/
integrity saga. corpus-bugfix back to normal triage.
