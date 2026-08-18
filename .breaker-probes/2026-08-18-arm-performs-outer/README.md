# The shadowing arm performs the effect it handles (2026-08-18)

- `pysh4.sexp` — the inner arm over E draws E while building its answer:
  (tick () s (resume (+ s (E.tick)) ...)) inside a shadowed handle. The
  draw routes to the OUTER arm — a handler arm runs OUTSIDE its own
  region (61 = 50 + outer's 10*s0+1; CPS-modeled, actual-cross-checked
  pre-pin). An arm that captured its own region would recurse forever —
  so this pin doubles as a non-termination guard. The xhs family pinned
  cross-handler arm performs (foreign effect); pysh4 pins the SAME-effect
  self-perform routing through a shadow. PASS x3 at 0c2b86ad3.
