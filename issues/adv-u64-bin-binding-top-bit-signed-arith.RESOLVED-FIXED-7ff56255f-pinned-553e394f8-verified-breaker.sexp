; FINDING (breaker, 2026-07-27): a `(bin (u64 n))` binding whose TOP BIT is set behaves as a
; SIGNED Int64 in runtime arithmetic, diverging from const UInt64 semantics. BOTH backends agree
; on the wrong value (consistent miscompile, not a divergence).
;
; Witness values, bytes [128,0,0,0,0,0,0,1] -> n SHOULD be 2^63 + 1 = 9223372036854775809:
;   (% n 1000)  runtime bin path: -807   (signed i64 -9223372036854775807 % 1000)
;               const UInt64 path: 809   ((% (: 9223372036854775809 UInt64) 1000) folds to 809) ✔
;   (/ n 2)     runtime: -4611686018427387903 (signed);  true UInt64 answer: 2^62 = 4611686018427387904
;   (Int64.of n) runtime: -9223372036854775807 — NO TRAP, silently the wrapped negative
;               (Int64.of of a genuine UInt64 > Int64.max should trap like the BigInt narrow pins)
;   control x=64 (top bit clear, 0x4000...001): runtime matches const (905) — only the top bit face.
;
; RELATED CONST FACE (supporting evidence, possibly same root): const `(+ (: Int64.max UInt64)
; (: 2 UInt64))` rejects CDZ0304 "the result overflows Int64" — UInt64 arithmetic misrouted
; through Int64 checked-add at const level too (a UInt64 add of those operands is fine: 2^63+1).
;
; Smell: the u64 segment binds raw i64 bits and downstream ops pick SIGNED opcodes (i64.rem_s,
; i64.div_s) instead of unsigned (rem_u/div_u) for a UInt64-typed value; Int64.of's range check
; likewise trusts the sign. wasm AND rust identical -> the bug is in shared lowering/typing of the
; u64 binding (n typed UInt64 but repped/op'd as signed), not a backend emit.
;
; GRADED REPRO (expected = TRUE UInt64 semantics; FAILS -807 vs 809 today on both backends):
(case "a u64 bin binding with the top bit set does unsigned arithmetic"
  (input  (do
        (def (main (: x UInt8))
          (do
            (def b (Bytes.of (list x 0 0 0 0 0 0 1)))
            (match b
              ((bin (u64 n)) (Int64.of (% n 1000)))
              (_ -1))))
        (export main)))
  (call   main (: 128 UInt8)) (output (: 809 Int64))
  (call   main (: 64 UInt8)) (output (: 905 Int64)))
