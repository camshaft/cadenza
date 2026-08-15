# cds1 — countdown alarm with snooze (2026-08-15, tick 1546)

(remaining, fire-count) 2-tuple: `tick` decrements answering the remaining
time, FIRING at zero — answering -(11+fires) and auto-RELOADING the seed
interval ((n%4)+3, recomputed in-arm at the reload site); `snooze` adds
slack. The shorter interval (3) fires MID-stream (row 6) where the longer
(5) fires on the very last tick — the fire sentinel migrates across the
packed total (…-11 at the end vs mid: 403050403020089 / 201030200890201),
and the post-fire reload rows prove the interval survives the fire.

2-branch arm, 2-tuple, envelope-safe. First model overflowed Int64 with
*1000 packing — repacked *100 with a compact sentinel. PASS ×3. **Pool —
fills hmg1/pkg1/cds1 (eighth trio ready).**
