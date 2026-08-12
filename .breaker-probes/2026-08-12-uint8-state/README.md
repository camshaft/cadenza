# 2026-08-12 narrow UInt8 handler state (tick 1336, base post-239 trunk)

- `uwr1.sexp` — the handler state is a NARROW UInt8: wrapping-add accumulates mod 256
  through the thread (100+200=300→44; 44+100=144 / 0+200=200; 200+100=300→44), each
  dispatch answers the Int64.of-widened running value. NO UInt8-typed handler state
  exists anywhere in 14* (narrow coverage there is op-ARG range checks in 14 part 1;
  wrapping coverage is Int64-width wa1/wa2 in 14c; UInt8 wrapping-add itself lives in
  06-numeric-model over params). Exercises adv-57's re-mask bug class THROUGH the
  state slot: a backend that skips the width re-mask on the threaded slot leaks 300.
  PASS ×3 (44144 / 200044).

## Tick 1337 addition
- `iwr1.sexp` — the SIGNED Int8 twin: wrapping-add crosses the sign boundary through
  the thread (100+100=200→-56 then -56+100=44; -28+100=72 then 72+100=172→-84), the
  Int64.of widening exposes the SIGN-EXTENSION (mask + sign-extend, not just mask —
  the signed face of the adv-57 re-mask class through the state slot). PASS ×3
  (-55956 / 71916). Batch-244 set: mts1, uwr1, iwr1.
