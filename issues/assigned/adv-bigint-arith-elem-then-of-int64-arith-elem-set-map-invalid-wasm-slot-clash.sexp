; BREAKER FINDING 2026-07-17 (trunk e47142e5d) — WASM-backend INVALID MODULE: a collection
; construction whose elements are (1) a BigInt ARITHMETIC result and then (2) `BigInt.of` over
; INT64 ARITHMETIC emits a function that fails wasm validation:
;     cdz: invalid component: failed to compile: wasm[0]::function[6]
;     wasm-tools: func 6 failed to validate: type mismatch: expected i64, found i32 (offset 0x223)
;
; MINIMAL (n=5):
;     (Set.len (Set.of (list (+ (BigInt.of n) (BigInt.of 1))    ; element 1: BigInt ARITH result
;                            (BigInt.of (+ n 2)))))             ; element 2: of(Int64 arith)
; The WAT shows the classic OPERAND-SLOT REUSE shape: the emitted main declares (local i32 i64 i32)
; and `local.tee 2` is used BOTH for an i32 BigInt heap handle (the `call 1` box result early on)
; AND for the i64 `(+ n 2)` overflow-guard temp later — one local slot, two value types.
;
; TRIGGER is exactly the ordered pair inside ONE collection construction:
;   works: each element alone; both-direct; both-arith; arith+of(direct-param); of(arith) FIRST then
;          arith SECOND (bn15/bn17 → correct 1/2); plain (list …) of the same pair (List.len → 2);
;          plain `=` equality of the same pair (0/1 correct).
;   fails: Set.of with [big-arith, of(i64-arith)] in THAT order (equal OR unequal values), and the
;          Map twin — (Map.insert (Map.insert (Map.empty) (+ (BigInt.of n) (BigInt.of 1)) 1)
;          (BigInt.of (+ n 2)) 2) → invalid function[7].
; Same failure at O0 and O2 (not an opt pass). rust twin: artifact references `cdz_num` (linkable
; only in-tree) so not independently graded here; wasm alone is definitive (an artifact that fails
; validation is a broken compile).
;
; Related family: adv-float-set-insert-emits-invalid-wasm-boxing-gap.RESOLVED / [[rcdzc-emit-direct-
; operand-slot-reuse]] — same locals-typing collision, new face: the BigInt box handle (i32) vs the
; deferred Int64 arithmetic temp (i64) inside a Set/Map element chain.
;
; Expected: Set of {n+1, n+2} has 2 elements -> 2 (and the Map twin -> 2).
(case "a set built from a BigInt sum and a BigInt.of over integer arithmetic has both elements"
  (doc    "`(Set.of (list (+ (BigInt.of n) (BigInt.of 1)) (BigInt.of (+ n 2))))` with n=5 holds the
           BigInt values 6 and 7 — two distinct elements, so Set.len = 2. Currently the wasm emit
           reuses one local slot for the first element's i32 heap handle and the second element's i64
           arithmetic temp, producing an INVALID module (func type mismatch expected-i64-found-i32)
           at every opt level; the same pair in a plain list or a bare `=` works, and the reversed
           element order works. The Map.insert twin fails identically.")
  (input  (do
            (def (main (: n Int64))
              (Set.len (Set.of (list (+ (BigInt.of n) (BigInt.of 1)) (BigInt.of (+ n 2))))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 2 Int64)))
