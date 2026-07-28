; FINDING (breaker, 2026-07-21): comparing two Float64.of-int conversions emits an INVALID
; wasm module when the LEFT operand converts a bare PARAM and the RIGHT converts an ARITH
; result — wasm-only (rust computes), O0..O3 stable:
;
;   (= (Float64.of-int n) (Float64.of-int (+ n 1)))   → INVALID MODULE (function[0])   ← BUG
;   (= (Float64.of-int (+ n 1)) (Float64.of-int n))   → computes (mirrored order OK)
;   (= (Float64.of-int n) (Float64.of-int m))         → computes (two params OK)
;   (= (Float64.of-int (+ n 1)) 101.0)                → computes (arith alone OK)
;   (= (Float64.of-int n) 100.0)                      → computes (param alone OK)
;
; Shape: the failing order evaluates the param-conversion FIRST, leaving its f64 on the stack
; while the RIGHT operand's inner (+ n 1) computes at i64 — likely a scratch/slot kind clash
; between the pending f64 and the i64 arith temp at the of-int call boundary (the same
; stack-discipline family as the fixed BigInt/Rational slot-clash guards in 19-sets). The
; mirrored order computes because the arith completes before any f64 is pending.
;
; Original context: probing the double-precision integer-boundary face
;   (= (Float64.of-int n) (Float64.of-int (+ n 1))) at n = 2^53 (adjacent ints collapse in
;   f64) — that semantic pin is blocked until this emit is fixed; witness case included.

(case "REPRO param-left arith-right of-int comparison computes"
  (input  (do
            (def (main (: n Int64))
              (= (Float64.of-int n) (Float64.of-int (+ n 1))))
            (export main)))
  (call   main (: 100 Int64)) (output (: false Bool)))

(case "WITNESS the 2^53 precision boundary (blocked on the emit fix)"
  (input  (do
            (def (main (: n Int64))
              (= (Float64.of-int n) (Float64.of-int (+ n 1))))
            (export main)))
  (call   main (: 9007199254740992 Int64)) (output (: true Bool))
  (call   main (: 100 Int64)) (output (: false Bool)))

(case "CONTROL the mirrored order computes today"
  (input  (do
            (def (main (: n Int64))
              (= (Float64.of-int (+ n 1)) (Float64.of-int n)))
            (export main)))
  (call   main (: 100 Int64)) (output (: false Bool)))
