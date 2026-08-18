# Let-bound replay value routes AND feeds both branches (2026-08-18)

- `pyr10.sexp` — (let ((r (resume ...))) (if (> r 15) (+ r s) (+ (* 2 r)
  (* 10 s)))): the binder is both the branch KEY and the surviving
  arithmetic's operand in each arm. Seeds split the INNER frame's branch
  (n=10 inner r=21 -> then-arm; n=0 inner r=10 -> else-arm doubling)
  while the outer frame holds steady (24 / 30, CPS-modeled + traced).
  Exercises 6c52dbc3c's let-init path with the binder consumed on BOTH
  sides of a data-divergent branch. Design note: two earlier drafts
  collapsed (constant-answer arms swallowed the branch signal; a
  threshold retune couldn't split the outer) — the fix was threading r
  itself through both arms so divergence SURVIVES the outer frame.
  PASS x3 at e11e4d3d8.
