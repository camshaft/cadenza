# PR #2293 review — xtask/src/fleet.rs (v-fleet-tooling) — OPEN — comment/code drift [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2293 (clear nudge-streak on a self-tick — hb newer than rearm —
not on absolute rearm-age; round-3 correction of the #2259/#2263 watchdog nudge-streak arc). Copilot 1
inline (id 3723850658, fleet.rs:3612, also line 11016).

## the CALL-SITE comment still describes the OLD `stale_after`-window rule, but the code now clears the streak on heartbeat-vs-rearm age + `interval` — stale/misleading for future maintenance (Copilot, fleet.rs:3612, also 11016) — comment/code-drift [VERIFIED, LOW]
> The comment still describes the old `stale_after`-based rule ("last re-arm older than a full stale_after
> window"), but the code now clears the streak based on heartbeat-vs-rearm age and `interval`. This is
> misleading for future maintenance/troubleshooting. This issue also appears on line 11016 of the same file.

VERIFIED against the #2293 diff. The fn doc for `should_clear_nudge_streak` IS rewritten (the "-"/"+" hunk:
old `stale_after`-window rationale removed, replaced with the round-3 "compare HEARTBEAT age to RE-ARM age
— they age in LOCKSTEP on the idle-re-arm path" explanation) and the signature changed to
`(rearm_age, hb_age, interval)`. BUT the CALL-SITE comment above the `if should_clear_nudge_streak(...)` is a
CONTEXT line (unchanged `//`), and it still reads: "Gate the clear on 'last re-arm older than a full
stale_after window' so a nudge-produced freshness does NOT wipe the accruing streak." The code beneath it now
passes `(rearm_age_secs(...), hb_age, interval)` — no `stale_after`. So the call-site comment describes the
superseded rule while the fn it calls implements the new one. Copilot flags the SAME stale wording at a
second call site (line 11016). LOW / comment-code-drift (behavior is correct — this is the watchdog fix
itself; only the two call-site comments lag the rewritten rule).

Fix per Copilot: update both call-site comments (~3612 and ~11016) to describe the heartbeat-vs-rearm
lockstep rule the code now uses (matching the rewritten fn doc), so a future maintainer troubleshooting the
escalation path isn't misled by the old `stale_after`-window framing. v-fleet-tooling owns xtask/src/fleet.rs.
PR OPEN → foldable pre-merge. (This is round 3 of the nudge-streak-clear discriminator — #2259 self-produced
freshness, #2263 the inline-comment stale_after fix, now #2293 the hb-vs-rearm lockstep correction; the
call-site comments just need to track the final rule.)
