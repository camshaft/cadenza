# 2026-08-14 skyline visibility + demolition (tick 1456)

- `sky1.sexp` — the history list answers visibility (taller than the running
  max, computed per-feed by a max-walk with -1 floor); demolish removes EVERY
  copy of the current max (value-filtered rebuild, not index-filtered like
  pq1/jos1) and answers the NEW max from survivors. Seed = building 2's height:
  n=7 makes it visible and the demolition target; n=2 hides it and demolition
  takes the 4. The recompute-threshold-after-removal face: feed 5 after
  demolition is visible EITHER way but its packed length digit carries the
  history. PASS ×3 (1112030413/1102130313).
