# Index-weighted post-resume toll (2026-08-18)

- `pyk1.sexp` — the toll is the PRODUCT of two captured tuple fields:
  (* 100 (* v k)) where k is the dispatch counter. Frame 1's toll is
  ZEROED by its own index (k=0), frame 2 pays v1*1 (441 = fold 41 + 400
  + 0 for s0=1). A product of captured fields distinguishes which
  frame's PAIR fed which toll beyond what either field alone could —
  cross-field capture consistency (pyr8 pinned both fields surviving;
  pyk1 pins them surviving TOGETHER, same-frame). PASS x3 at e4bf6e301.
