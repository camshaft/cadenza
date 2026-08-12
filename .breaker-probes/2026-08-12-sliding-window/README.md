# 2026-08-12 sliding-window dedup (tick 1357, base post-#22-fix trunk)

- `swd1.sexp` — last-3 sliding-window state: feed answers membership (recursive
  has-walk) then slides via List.push + drop-head (`(list _h .. t)` cons-match).
  Membership verdicts FLIP as elements age out: n=3 → 00101 (the second 3 ages
  out between hits), n=5 → 00111 (5 collides with the literal feed so the window
  saturates). Two findings from the draft: id sw1 TAKEN in 14c (renamed swd1);
  and a `(list _h .. t)` match with no wildcard arm under an if is a REFUTABLE
  match → CDZ0210 clean decline — the fold wants the irrefutable wildcard arm
  even when the guard makes the cons-arm exhaustive at runtime. PASS ×3.
