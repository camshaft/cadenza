# DECLINE FACE: branch-routed abort (2026-08-18)

- `pyi5.sexp` — the body's if (on a drawn answer) routes to an aborting
  op at different depths per path: DECLINES uniformly at the
  tail-resumptive fold boundary ("later increment" diagnostic class).
  Also declines: the single-arm mixed abort/resume gated on state
  (pyi4-shape, pya1's sibling). The straight-line control (two ticks
  then stop, no branch) PASSES — that shape is corpus-pinned as abm1.
  So the fold's abort support is straight-line-only today: any BRANCH
  choosing whether/when to abort joins the later-increment watch.
  Flip oracles hand-modeled and DIVERGENT: n=10 -> 9030 (s0=1: a=11,
  not >15, tick2 advances to s=3, stop aborts 9000+30); n=0 -> 9020
  (s0=0: a=1, tick2 to s=2, stop 9000+20). Decline-witness only — no
  baseline row until the fold increment lands.
