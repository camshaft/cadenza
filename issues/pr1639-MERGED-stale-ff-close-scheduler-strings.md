# PR #1639 review comment — xtask/src/fleet.rs (v-fleet-tooling) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1639 (MERGED). #1575/#1532 model-correction lineage.

## User-facing scheduler strings still say FF-trunk + close, contradicting the cherry-pick + no-auto-close model (Copilot, fleet.rs:735, also :7772) — doc/UX [VERIFIED]
> The SchedulePass help text is updated to the cherry-pick model, but some user-facing scheduler OUTPUT
> still says the old fast-forward/close behavior (`schedule-plan` prints "LAND (merged → FF trunk + ack
> merged)" and "REJECT (… → ack reject + close)", the executor logs "trunk FF FAILED").

VERIFIED against the merged code: fleet.rs:8232 `ReapAction::LandMerged => "LAND (merged → FF trunk + ack
merged)"`, :8236 `ReapAction::Reject => "REJECT (required check RED → ack reject + close)"`, :8523 `"…
merged on GitHub but trunk FF FAILED ({e})…"`. These are user-facing strings still describing the OLD
fast-forward-trunk + auto-close model, contradicting the cherry-pick-mergeCommit.oid + no-auto-close model
your #1532/#1575 corrections established. Update: "FF trunk" → "cherry-pick mergeCommit onto trunk",
"ack reject + close" → "ack reject" (no auto-close), "trunk FF FAILED" → "trunk cherry-pick FAILED". Two
regions (:8232-8236 the ReapAction labels + :8523 the executor log; also :7772 doc). MED — operators read
these strings; stale wording misleads on what the scheduler actually did. Fix-forward.
