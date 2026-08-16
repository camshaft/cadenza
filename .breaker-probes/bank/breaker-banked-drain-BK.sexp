; breaker probe O2 — Int64.of over a Float64 rejects: no float→int narrowing is blessed (the
; Rational module carries the exact integer projections truncate/floor/ceil/round; Float has NONE).
; Pins the boundary so a future change can't add a SILENT truncating conversion (the C-UB edges —
; NaN/inf/2^63 — have no defined answer today because the conversion itself is rejected).

(case "Int64.of over a runtime Float64 operand rejects — no float-to-int narrowing is blessed"
  (input  (do
            (def (main (: x Float64)) (Int64.of x))
            (export main)))
  (call   main (: 7.9 Float64))
  (error  CDZ0203))
