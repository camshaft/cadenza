# 2026-08-12 cursor-stack (tick 1342, base post-241 trunk)

- `cst1.sexp` — state (buf: List Int64, top: Int64) as a CURSOR-stack: push does
  List.update at the cursor when a STALE slot exists (overwriting a previously
  popped value in place) or List.push when at capacity; pop decrements and reads
  buf[top-1] via Option-match; the over-pop answers -1 defensively without touching
  the state. Distinct from mns1 (grow-only min-stack): this exercises List.update
  IN AN ARM as an in-place overwrite of a persistent list cell that an earlier
  dispatch wrote — RRB path-copy correctness through the state thread. PASS ×3
  (1204250031 / 1207250061).
