; FINDING (breaker, 2026-07-20): a RECURSIVE fn over a mixed BigInt/Int64 signature whose
; recursive-call RESULT is locally BOUND and fed into further BigInt ops breaks THREE ways
; depending on spelling - a compiler HANG (>100s CPU-bound, no output) and two BOGUS diagnostics.
; The non-recursive twin compiles+runs fine, so it's recursive-result inference/lowering.
;
; MATRIX (all: (def (f (: base BigInt) (: e Int64) (: md BigInt)) ...) with main narrowing to
; Int64; the binding hh = the recursive result):
;   HANG    (do (def hh (f base (- e 1) md)) (% (* hh hh) md))    - do-def + hh used TWICE (c6)
;   HANG    same with (/ e 2) self-arg (c4)                        - 100s+ CPU-bound, killed
;   CDZ0201 (do (def hh (f base (/ e 2) md)) (% hh md))            - do-def + hh used ONCE:
;           "member access requires a record, found Type" AT THE RECURSIVE CALL SITES (bogus)
;   CDZ0301 (let ((hh (f base (- e 1) md))) (% (* hh hh) md))      - let-form: "no implicit
;           conversion between Int64 and BigInt" at the let body (hh IS BigInt - bogus), and an
;           explicit (: ... BigInt) result annotation does NOT clear it
;   ok      recursion w/o binding the result ((f base (- e 1) md) in tail position)
;   ok      NON-recursive helper, same bind-square-mod shape (c9)
;   ok      recursive + do-def result but scalar-only signature (earlier corpus pins)
;
; So: recursive-RESULT type flows wrongly once the result is locally re-bound in a mixed-numeric
; recursive fn - sometimes diverging (occurs-check/instantiation loop?), sometimes resolving the
; fn name to a Type (the CDZ0201 spelling), sometimes defaulting hh to Int64 (the CDZ0301 one).
; IMPACT: modpow/repeated-squaring - a core BigInt idiom - cannot be written recursively; the 06-
; corpus modpow pin is Int64-only, which is why this was never hit.
;
; Repro = the CDZ0201 spelling (fast, deterministic). The HANG spelling is c4/c6 above - do not
; add to a graded corpus until fixed (it would hang the gate).
(case "a recursive BigInt fn whose bound result feeds a mod compiles (FINDING repro - bogus CDZ0201 today)"
  (input (do
        (def (f (: base BigInt) (: e Int64) (: md BigInt))
          (if (= e 0)
              base
              (do
                (def hh (f base (/ e 2) md))
                (% hh md))))
        (def (main (: e Int64))
          (Int64.of (f (BigInt.of 7) e (BigInt.of 100))))
        (export main)))
  (call main (: 8 Int64)) (output (: 7 Int64)))
