# The shadowing arm performs the effect it handles (2026-08-18)

- `pysh4.sexp` — the inner arm over E draws E while building its answer:
  (tick () s (resume (+ s (E.tick)) ...)) inside a shadowed handle. The
  draw routes to the OUTER arm — a handler arm runs OUTSIDE its own
  region (61 = 50 + outer's 10*s0+1; CPS-modeled, actual-cross-checked
  pre-pin). An arm that captured its own region would recurse forever —
  so this pin doubles as a non-termination guard. The xhs family pinned
  cross-handler arm performs (foreign effect); pysh4 pins the SAME-effect
  self-perform routing through a shadow. PASS x3 at 0c2b86ad3.
- `pysh5.sexp` — REPEATED self-performs: two shadowed draws, each arm
  activation self-performing to the OUTER handler. Outer ladder advances
  once per inner dispatch (s0, s0+1) while the inner state doubles
  independently (50, 100). Both ladders land in both answers (121061 =
  a 61 + 1000*b 121... model: a=50+11=61, b=100+21=121 for s0=1). Either
  ladder stalling or a self-perform re-entering the inner region
  misprices a distinct digit range. PASS x3 at fd51d1f2b.
- `pyi3.sexp` — the self-perform hits a TOLLED outer arm: the outer
  dispatch triggered from INSIDE the inner arm wraps its x1000 toll
  around everything downstream (the inner region's completion included),
  and two outer frames stack (5521 / 3510, CPS-modeled: body 41+510,
  tolls 3000+2000 for s0=1). Composes pysh4's routing law with pyr1's
  toll law across the arm boundary — the toll of a dispatch RAISED BY AN
  ARM scopes like any other. PASS x3 at 942944f3f.
