; FINDING (breaker, 2026-07-28): the CONST-EVALUATOR twin of the fixed u64-binding bug — after
; v-core-opt 7ff56255f the RUNTIME u64 top-bit binding computes unsigned (verified 809/905), but a
; CONSTANT-FOLDABLE (bin (u64 m)) over a top-bit byte literal still treats m as SIGNED Int64:
;
;   F1 wrong value : (match (Bytes.of (list 128 0..0 9)) ((bin (u64 m)) (if (> m 5u64) 1 0)))
;                    -> folds to 0 (signed negative < 5); the RUNTIME twin (x=128 entry arg)
;                    computes 1. Side-by-side witness: 10*const + runtime = 01 -> printed 1.
;   F2 bogus CDZ0304: const (% m 1000u64) -> "constant arithmetic operation overflows its
;                    integer type" (2^63+9 % 1000 = 817 is fine unsigned)
;   F3 bogus CDZ0302: two const 8-byte matches in one fn -> "integer literal does not fit its
;                    width" (the folded 2^63+9 re-materialized as a SIGNED-width literal?)
;
; All three = the const evaluator binds/folds the u64 segment through Int64. Same root as the
; fixed runtime face; the fix needs mirroring in eval/const-fold (and the re-materialization of
; folded UInt64 values, F3's literal-width check).
;
; GRADED REPRO (side-by-side const-vs-runtime; FAILS wasm+rust today at mode 1 (0, want 1);
; runtime control mode 2 already right):
(case "a constant-folded u64 bin binding with the top bit set compares unsigned"
  (input  (do
        (def (main (: mode Int64))
          (if (= mode 1)
              (match (Bytes.of (list 128 0 0 0 0 0 0 9))
                ((bin (u64 m)) (if (> m (: 5 UInt64)) 1 0))
                (_ -2))
              (match (Bytes.of (list (UInt8.wrap (* mode 64)) 0 0 0 0 0 0 9))
                ((bin (u64 m)) (if (> m (: 5 UInt64)) 1 0))
                (_ -2))))
        (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64))
  (call   main (: 2 Int64)) (output (: 1 Int64)))
