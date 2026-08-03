# PR #1340 review comment — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1340 (PR: "cand: v-fleet-tooling — 270d0470a").
Continuation of the #1260 "dispatch nothing" operator-message thread.

## "dispatch nothing" message doesn't cover the empty-queue / no-capacity case (Copilot, fleet.rs:7432) — doc/UX
> The "dispatch nothing" message is misleading: `picks.is_empty()` can also occur when there are no
> queued MRs (or when `cap.saturating_sub(current_in_flight)` yields 0). The output should include
> the empty-queue case so operators don't infer collisions when the queue is simply empty.

`picks.is_empty()` has three causes — no queued MRs, no free capacity (`cap - in_flight == 0`), and
all-file-collide — but the message only describes the collision case. Distinguish them (or at least
mention empty-queue / no-capacity) so an operator doesn't misread an empty queue as a collision
stall. Builds on the #1260 message rewording.
