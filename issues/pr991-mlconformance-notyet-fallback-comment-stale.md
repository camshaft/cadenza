# PR#991 review comment — report_ml_conformance not_yet fallback comment "shouldn't happen" stale after wall-clock deadline (v-fleet-tooling)

Mirrored from GitHub PR#991 review comment (Copilot), id `3695317737`.
File: `xtask/src/main.rs:4163` — gate harness → v-fleet-tooling. Blame `fb6c17ead` "xtask: refactor
report_ml_conformance to RETURN its classification + add covered-subset selection".

## Comment (verbatim)

- (id 3695317737, xtask/src/main.rs:4163) "The fallback arm comment says an unfilled slot 'shouldn't
  happen', but with the new wall-clock deadline behavior unfilled slots are expected when the run times
  out (and are intentionally counted as `not_yet`). Updating the comment avoids misleading future
  readers."

### Liaison verification (confirmed on trunk 18dba958f)

main.rs:4159: `// NotYet or an unfilled slot (a worker that never reached it — shouldn't happen) → not-yet.`
then `_ => not_yet += 1`. With the wall-clock-deadline behavior (the differential is time-bounded for
fleet safety, per the report doc), a run that TIMES OUT leaves slots UNFILLED — which this arm
intentionally counts as `not_yet`. So "shouldn't happen" is now wrong: an unfilled slot is EXPECTED on
timeout, and counting it `not_yet` is deliberate (not a can't-happen defensive fallthrough). Reword to
"…or an unfilled slot (a worker cut off by the wall-clock deadline — EXPECTED on timeout) → not-yet".
Comment-only, behavior-neutral.

Owner: **v-fleet-tooling** (`xtask/src/main.rs` gate harness; `fb6c17ead`). Reword the "shouldn't happen"
to reflect the expected-on-timeout unfilled slot.
