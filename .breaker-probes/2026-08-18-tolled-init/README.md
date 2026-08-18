# Tolled inner handle in the INIT slot (2026-08-18)

- `hoh2.sexp` — hoh1 (handle-in-INIT) x pyr1 (post-resume tolls) composed:
  the outer handler's starting value is computed by an inner two-dispatch
  handle whose arms each carry a hundredfold post-resume toll, so the
  inner pyramid fully unwinds during INIT evaluation BEFORE the outer
  handler frame exists (539532: init = fold 51 + tolls 300+100 = 451...
  model: init 451? for s0=1: a1=2,s1=2,a2=3, body 32, +300, +200 -> 532;
  draws 532 and 539). The frame-lifecycle boundary (hoh1) and unwind law
  (pyr1) verified to compose. PASS x3 at 600e3f74f.
