; FINDING (breaker, 2026-07-28): UInt64→BigInt widening DIVERGES between backends on top-bit
; values — wasm widens by VALUE (unsigned), rust SIGN-EXTENDS the i64 carrier.
;
;   (bin (u64 n)) over [128,0,0,0,0,0,0,9] -> n = 2^63 + 9 (runtime binding, post-7ff56255f)
;   (% (BigInt.of n) 1000N):
;     wasm       -> 817   (BigInt.of n = +9223372036854775817)  CORRECT
;     rust       -> -799  (BigInt.of n = -9223372036854775799 — sign-extended)
;     rust-async -> -799
;   x=0 control (n=9) -> 9 on all three.
;
; Fourth member of the u64-binding family: runtime div/rem FIXED (7ff56255f), const folds OPEN
; (#30), and now the BigInt WIDENING wrong on the rust EMIT specifically (wasm right, so the
; shared lowering is fine — the rust backend's UInt64→BigInt conversion re-reads the carrier as
; i64; likely `as i64` → BigInt::from instead of `as u64` → BigInt::from at the emit seam).
; This is the escape-hatch path 8-byte-id/hash code takes for wide arithmetic — silently wrong
; SIGN on the whole upper half of u64, rust targets only.
;
; GRADED REPRO (= post-fix pin; wasm passes today, rust FAILS -799):
(case "a top-bit u64 binding widens to BigInt unsigned"
  (input  (do
        (def (main (: x UInt8))
          (match (Bytes.of (list x 0 0 0 0 0 0 9))
            ((bin (u64 n)) (Int64.of (% (BigInt.of n) (BigInt.of 1000))))
            (_ -2)))
        (export main)))
  (call   main (: 128 UInt8)) (output (: 817 Int64))
  (call   main (: 0 UInt8)) (output (: 9 Int64)))
