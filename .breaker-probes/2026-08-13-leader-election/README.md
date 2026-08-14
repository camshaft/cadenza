# 2026-08-13 bully leader election (tick 1446)

- `led1.sexp` — candidate Set (seeded with sentinel 0, lengths answered -1 to
  hide it): elect scans Set.to-list for the max id; the elected leader's id is
  THREADED THROUGH THE BODY into dereg (answer→argument dependency), forcing
  re-election from survivors. Seeds: n=9 elects 9 then falls back to 5; n=3
  elects 5 then falls back to 3 — different leaders AND different survivors.
  Set-enumeration max-scan + the depose-and-reelect protocol. PASS ×3
  (12309205/12305203).
