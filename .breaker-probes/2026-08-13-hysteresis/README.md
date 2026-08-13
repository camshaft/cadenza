# 2026-08-13 hysteresis gate (tick 1439)

- `hys1.sexp` — the two-threshold gate: output ON at >=8, OFF at <=3, HELD in
  the (3,8) band. The pin's teeth: the SAME mid-band feed (5 or 6) answers
  differently depending on which side entered the band — n=9 arrives high so
  the first 5 holds ON (11); n=5 starts mid-band from the OFF seed (01). The
  answer packs the output bit and an in-band marker (independent `and`
  comparison). One-line nested-if transition whose ELSE leg is the identity
  hold — the memory-in-the-else shape. PASS ×3 (10110001/1010001).
