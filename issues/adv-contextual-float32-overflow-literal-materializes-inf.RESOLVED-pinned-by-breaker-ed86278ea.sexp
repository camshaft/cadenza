; FINDING (breaker, 2026-07-21): a Float32-overflowing literal grounded CONTEXTUALLY (through
; an arith op's width unification, no annotation anywhere) silently materializes ±inf on BOTH
; backends — while the INTEGER analogue of the same shape rejects.
;
;   (def (main (: a Float32)) (+ a 1.0e300))   → RUNS: main 0.5 = inf   (wasm AND rust, O0..O3)
;   (def (main (: a UInt8))   (+ a 10000))     → rejects CDZ0201 (the arith-spine contextual
;                                                 range check, fixed earlier for integers)
;   (: 1.0e300 Float32)                        → rejects CDZ0302 (direct annotation)
;   (: (if c 1.0e300 0.0) Float32)             → rejects CDZ0302 (ffa733b67 branch descent)
;
; The `+` unifies its operand widths, grounding `1.0e300` at Float32 — where it overflows
; binary32 and becomes inf, "a malformed value with no written form" (ffa733b67's own words).
; The integer arith-spine check climbs the spine and rejects contextually-grounded overflows;
; the FLOAT path has no such check, so the literal converts f64→f32 saturating to inf with no
; error. Fourth member of the width-check gap family this arc (fitting-branch invalid module /
; const-folded inf / record+user-sum escape / contextual-float inf) — all one audit surface:
; every path that grounds a float literal at Float32 must run the fits_f32 check the direct
; and branch paths run.
;
; Grading note: with an (error …) expectation this case grades TODO (a codeless check-decline
; on the reject side), but an (output …) expectation shows it RUNS to inf — use the runtime
; witness below to see the live behavior.
;
; Expected: reject (CDZ0201 contextual, matching the int arith-spine precedent).

(case "REPRO a Float32-overflowing literal grounded through an arith op is rejected"
  (input  (do
            (def (main (: a Float32))
              (+ a 1.0e300))
            (export main)))
  (error  CDZ0201))

(case "WITNESS today it runs to inf (delete when the reject lands)"
  (input  (do
            (def (main (: a Float32))
              (+ a 1.0e300))
            (export main)))
  (call   main (: 0.5 Float32)) (output (: 0.5 Float32)))

(case "CONTROL the fitting contextual literal computes"
  (input  (do
            (def (main (: a Float32))
              (+ a 1.5))
            (export main)))
  (call   main (: 2.25 Float32)) (output (: 3.75 Float32)))
