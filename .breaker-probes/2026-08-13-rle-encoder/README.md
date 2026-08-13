# 2026-08-13 run-length encoder (tick 1415)

- `rle2.sexp` — the ENCODER direction (rle1 pinned the run TRACKER — last/run/
  best scalars): the state is the encoded (value,count) pair LIST itself; an
  equal-to-last feed bumps the tail pair via List.update at len-1 (tuple rebuilt
  inside), a fresh value pushes (v,1). Seed 5 makes the middle literal MERGE
  into one five-long run (list stays len 1); seed 3 alternates to three pairs
  with the last reaching count 2. Update-or-push routed by a comparison against
  a TUPLE FIELD of the tail element. PASS ×3 (1112213132/1112131415).
