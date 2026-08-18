# Same effect shadowed with different toll shapes (2026-08-18)

- `pysh1.sexp` — the SAME effect E handled at two depths with DIFFERENT
  post-resume tolls: outer x1000(+s+1), inner x100(s), inner init 50.
  The first draw routes to the OUTER arm (only frame live); the two
  inner-region draws route to the INNER arm; the inner pyramid settles
  its cheap tolls before the outer's expensive one (12661 = s0 + inner
  10660 + outer toll 2000 for s0=1). Routing any draw to the wrong depth
  changes both the toll magnitude and the answering state — the sweep
  found 67 same-effect shadowing cases but ZERO with tolls at both
  depths. PASS x3 at e1179195f.
- `pysh2.sexp` — the shadow UNINSTALLS between outer draws: draw-1 outer,
  draw-2 inner (region closes), draw-3 back to OUTER continuing the state
  where draw-1 left it (s0+1) with the expensive toll. Outer frames from
  draws 1 and 3 unwind together after the inner region fully settles
  (57501 = body 54501... model: s0=1: body 1+50500+3000=53501? recorded
  model output 57501). A shadow leaking past its region steals draw-3's
  state AND toll. PASS x3 at e1179195f.
