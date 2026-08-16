# FINDING #20 (2026-08-12): let-bound state-Bytes + decode match ICE

Trigger (1 op, 1 dispatch — minimal): in a tail-resumptive arm,
  (let ((b (Bytes.of (list (UInt8.wrap s)))))
    (resume (match (String.from-bytes b) ...) s))
ICEs "parameter reference has no local slot" — uniform x3 (wasm hard error;
rust targets grade todo on the same compile error). The #13/tk3d spec-body
share class.

Controls:
- constant byte in the same let: compiles (fb4)
- SAME expression INLINE (no let): compiles + correct values 1/-1 (fb5)

Filed: adv-let-bound-state-bytes-from-bytes-no-local-slot.sexp (queue) with
the inline twin's values. The drawn-byte UTF-8 validity probe (fb, 2-op)
that found it is the outer shape; fb5-inline stays a pin candidate NOW.

## Scope sweep (tick 1296): MUCH broader than Bytes
ICE (all "no local slot"): let+from-bytes-match (fb3) · let-List+List.at-match
(g20a) · let-Bytes+Bytes.at-match (g20b) · SCALAR-let+List.at-match (g20d) ·
scalar-let+USER-SUM-match (g20f).
COMPILE: no-let Option-match (g20e) · let + non-match consumer List.len (g20c).
GATE FINAL: ANY let preceding ANY sum-match in the arm's RESUME VALUE.
(bind-then-branch — a very common user shape.) Reported; fix surface = the
let-wrapped-match resume-value path in the arm fold.

## RECLASSIFIED (tick 1297): DECLINE not ICE — my severity call was wrong
The gate grades the whole class TODO (0/1/0) and cdz exits 1 with a plain
error line — no panic. "parameter reference has no local slot" is a CODED
decline (infer.rs/db.rs) reached via the deep-fresh-copy guard (the actual
#13 crash-protection working as designed). Finding-20 = a precision-fold
TODO on let-wrapped-match resume values (v-effects fold lane), NOT a bug.
LESSON: decline-vs-ICE is a GATE-VERDICT question, not an error-TEXT
question — the message text matching an old crash's text means nothing.
fb5-inline stays a pin candidate; fb3/g20* become flip-on-fold witnesses.
