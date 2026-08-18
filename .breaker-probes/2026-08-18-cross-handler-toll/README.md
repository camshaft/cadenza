# Cross-handler post-resume toll (2026-08-18)

- `pyt1.sexp` — the post-resume toll PERFORMS A FOREIGN EFFECT: inner E's
  arm is (+ (resume s (+ s 1)) (T.levy)) with T handled OUTSIDE E. Sweep:
  zero corpus cases perform in post-resume position (the xhs family
  performs BEFORE resuming; pyr tolls are pure). The levies fire during
  the innermost-first unwind, so outer T's counter advances in UNWIND
  order, not dispatch order — main(10): body 21, dispatch-2's toll levies
  t=1 first, dispatch-1's levies t=6 second -> 21+1+6=28. A lowering that
  hoists the levy before the resume, or runs tolls in dispatch order,
  changes the answer. Two ids collided before pyt1 (tax1 tax brackets,
  tol1 toll plaza — free-id grep caught both). PASS x3 at 4b6404cae.
- `pyt2.sexp` — the DISPATCH-ORDER mirror: (resume (+ s (T.levy)) (+ s 1))
  levies while BUILDING the answer, so levies fire at dispatch (t=0 then
  t=5 for main(10): a=1+0... wait, model: a = s + levy = 1 + t0, b = 2 +
  t1). The pyt1/pyt2 pair pins the timing law: argument-position performs
  PRECEDE the suspend (dispatch order), post-resume performs FOLLOW the
  replay (unwind order). Same machine, opposite orders, different answers
  (82/71 vs 28/26). PASS x3 at 4b6404cae.
- `pyt3.sexp` — the levy in the NEXT-STATE argument: (resume s (+ s
  (T.levy))). DECLINES uniformly x3: "this handler is not yet reducible by
  the tail-resumptive fold (cross-function or non-tail resume arrives in a
  later increment)". Sharp asymmetry vs pyt2: a foreign perform in the
  ANSWER argument folds, the same perform in the STATE argument does not —
  the state-thread expression is the discriminator. Held as todo-witness
  (safe decline, no wrong answer); oracles hand-modeled for the flip
  (main(10)=21, main(0)=11). NOT filed as an issue — the diagnostic
  explicitly says "later increment" (known roadmap), unlike pyr3's
  unexpected gap.
