# Post-resume toll that traps (2026-08-18)

- `pyt4.sexp` — the toll divides by the captured pre-resume state:
  (+ (resume s (+ s 1)) (/ 100 s)). Zero seed: the FIRST frame's toll
  traps at UNWIND — after the whole body already ran (both dispatches
  complete, all state threading done). Nonzero seed: both quotient tolls
  pay cleanly (171 = 21 + 50 + 100). Pins toll evaluation TIMING via the
  trap: a lowering that evaluates the toll before the resume would trap
  before the body ran; one that skips a trapping toll would return 21.
  First trap-outcome case in the post-resume family. PASS x3 (incl. the
  trap rows) at fd51d1f2b.
- `pyt5.sexp` — the mirror: the BODY traps under a PENDING toll. The
  replayed continuation divides by the drawn answer; the zero seed traps
  INSIDE the resumed rest-of-body while the frame's x1000 toll is still
  pending — the trap wins, the toll never lands. Nonzero: 2601 = body
  601 + toll 2000. With pyt4 (toll traps after body) the pair covers
  both directions of trap-vs-pending-work ordering. PASS x3 at fd51d1f2b.
- `pyt7.sexp` — the INIT traps in VALUE position: (handle E (/ 60 (% n
  3)) ...). Zero seed traps BEFORE any frame installs or dispatch runs
  (nonzero: 121670, model-verified). Completes the trap-position triple:
  INIT trap = eager, DISCARDED pure trap = elided (dsc1/pyt6), TOLL trap
  = at unwind after the body (pyt4). PASS x3 at fd51d1f2b.
