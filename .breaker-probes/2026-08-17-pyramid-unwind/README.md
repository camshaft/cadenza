# Pyramid unwind, post-resume arithmetic family (2026-08-17)

Sweep across ALL 1481 resumes in 14c found EVERY one in tail position —
zero arithmetic / if / match / let on resume's return value. This bank
maps the family.

- `pyr1.sexp` — ADDITIVE tolls: (+ (resume s (+ s 2)) (* 1000 (+ s 1))).
  Three dispatches, tolls unwind innermost-first, each keyed to state AT
  ITS OWN DISPATCH. PASS x3.
- `pyr2.sexp` — MULTIPLICATIVE tolls: (* (resume ...) (+ s 2)). The product
  pins the unwind PAIRING (which factor saw which intermediate) beyond what
  addition distinguishes. PASS x3.
- `pyr4.sexp` — IF CONDITION DIRECTLY ON THE RESUME CALL: (if (> (resume
  ...) 35) ...). Outer frame always takes the OPPOSITE branch from the
  inner; seeds flip the inner branch -> thousandfold-apart answers. PASS x3.
- `pyr3.sexp` — LET-BOUND resume result reused in both if branches:
  (let ((r (resume ...))) (if (> r 35) ...)). DECLINED all 3 backends
  (uniform): "resume outside a lowered handler arm is not yet realized" —
  the let-binding hides resume from the arm-tail lowering pattern. HELD as
  todo-witness; the shape is semantically reasonable (deep-handler resume
  has a value) so this is a real expressiveness gap, not nonsense.
  Diagnostic recovered via `cdz compile` on the extracted (do ...) with a
  .sexp extension (gate's todo verdict prints no reason; --case prints the
  program but not the diagnostic either).
- `pyr5.sexp` — MATCH SCRUTINIZING THE RESUME CALL: (match (% (resume ...)
  3) ...) across three literal-guard arms. PASS x3 — sharpens the pyr3
  boundary: if-condition AND match-scrutinee positions see through to the
  resume, only the LET binder blocks arm lowering.
- `pyr6.sexp` — LET-BOUND resume into a PURE COMBINE (+ (* 2 r) s), no
  branch at all — the minimal binder-only shape. DECLINES pre-fix
  (confirms v-effects' root cause: ANY let-bound resume in a multi-perform
  body, not just branched consumers). Second flip-witness for f98ce59f7
  alongside pyr3; oracle main(10)=89, main(0)=42 (hand-modeled).

UPDATE (tick 1741): pyr3 fix landed as 6c52dbc3c (rcdzc/effects two-hole
refold now folds a let-bound resume RESULT). pyr3 + pyr6 FLIP TO PASS;
full family 6/6 on wasm/rust/rust-async with fresh worktree binaries.
pyr3/pyr6 promote as pass-witnesses in batch-317.
