# Fleet-health observation (corpus-bugfix 2026-07-19 ~10:19+ trunk 540c30df6)

23 agents ⚠STALE (3-8h HB) while ~14 are fresh (seconds-old). Trunk has NOT advanced in ~4h; my two
pending doc-fix MRs (pr633#1 7251a5c77, pr634 e2ea45a6f) are stuck behind it.

## Signals
- pr-sync: 4h stale, 165-msg queue, ⚠STALE. Its "↳ Did" = it SELF-ESCALATED a `pr-sync-BLOCKED` to the
  operator (recursive-generic stand-down). => that block is OPERATOR-OWNED; NOT duplicating it.
- concierge: 4h stale ⚠STALE — the agent that normally hand-arms stalled /loop crons is ITSELF down.
- v-fleet-tooling: 8h stale, 3 unread ⚠STALE — owns the loop tooling / window.sh; can't fix its own cron.
- Also 3-4h stale: v-effects, v-compiler-ml, fuzzer, reviewer, v-guide, v-diagnostics, v-quantity,
  v-metaprogramming, v-memory-safety, v-guide-infra, v-lsp, v-property-testing, v-slack-bridge (1d).

## Hypothesis
LIKELY a cascade of pr-sync's deliberate stand-down (awaiting operator on recursive-generic): downstream
verticals finish work, MR, then idle because landings are frozen → HBs go hours-stale. If so, it's
operator-owned + self-healing once pr-sync unblocks. LESS likely but possible: a loop-stall wave (shared
window.sh/cron), given v-fleet-tooling (loop-infra owner) is the worst at 8h.

## Action taken
NOT duplicating pr-sync's operator-owned block. Sent ONE fleet-health ask to concierge (drains on re-arm)
flagging: (a) concierge-itself-stale = recovery mechanism down, (b) v-fleet-tooling 8h (shared loop infra),
(c) the wide stale cohort — for hand-arming once pr-sync unblocks. corpus-bugfix itself is LIVE + functional
(triage unaffected; only my MRs' LANDING is blocked, downstream of pr-sync, out of my hands). Continuing.

---
## ROOT CAUSE (concierge answer, 2026-07-19) — it was the RECURRING HOST BEDROCK 403 CRED EXPIRY
NOT a pr-sync stand-down cascade, NOT a loop-stall wave. The host's creds lapsed ~2pm → every agent hit
`API Error 403 … not authorized for bedrock:InvokeModelWithResponseStream / Please run login` → whole cohort
stale at once + trunk flat ~4h. This is the KNOWN recurring-daily stall ([[fleet-wide-403-bedrock-auth-stall-recurs-daily-pr-sync-please-run-login]]).
FIX: operator refreshed the credential; concierge re-armed pr-sync (trunk integrating again) + hand-armed
v-fleet-tooling (genuinely 9h-idle — my non-duplicative signal #2 was RIGHT). concierge back live (signal #1
addressed). Rest of cohort self-healed on auth return. My pending MRs will land as pr-sync drains the backlog.
LESSON: I missed cred-403 in my differential (guessed stand-down/loop-wave); the "all-stale-at-same-age,
no-single-locus" signature = cred-403 FIRST. Folded into the existing memory note. This file CLOSED.
