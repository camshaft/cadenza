# Silent post-resume levy (2026-08-18)

- `tmb2.sexp` — the tombstone (tmb1) and cross-handler toll (pyt1)
  composed: each inner arm replays the tail, then levies the outer
  handler DISCARDING the value, then answers a tombstone. The levies are
  observable ONLY through the outer audit read after the inner handle
  completes (4111 = inner 41 x100 + audit t0+10). A dead-code elision
  that drops the valueless levy shifts the audit by 10 while leaving the
  inner digits untouched — the split oracle localizes the failure. Also
  exercises CDZ0307 (discard warning fires on the levy, correctly).
  PASS x3 at 600e3f74f.
