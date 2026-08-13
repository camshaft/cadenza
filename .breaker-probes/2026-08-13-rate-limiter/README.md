# 2026-08-13 sliding-window rate limiter (tick 1407)

- `rlm1.sexp` — allow(t) PRUNES expired timestamps (rebuild-filter via recursive
  push-if walk into a fresh annotated empty list) then admits under the cap
  (2 per 10 ticks); rejected requests still thread the PRUNED list (the prune
  is not rolled back on bounce). The seeded third request: t=13 bounces inside
  the window (both slots taken) so 25/26 both land; t=22 lands (10/11 expired)
  and steals the slot that would have admitted 26. Composes prune-rebuild +
  cap-check + reject-still-mutates in one arm; swd1's window is count-based —
  this one is VALUE-based (timestamp cutoff). PASS ×3 (22122/22221).
