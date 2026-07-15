;; ✅ FIXED by the seed (`spec`@`b2bf850d` "a br_table match arm branches to the match's own join block,
;; not one block past it", 2026-07-14) — reported by this loop, fixed within one iteration. `go(4)` now
;; correctly returns "bb" (byte-len 2). KEPT as a regression witness: run `cdz compile` on this file and
;; check `go(4)` == 2. (The port's src/print.cdz reverted its if-chain `digit` workaround to a clean
;; 10-arm `match` once this landed.) Original report below.
;;
;; MISCOMPILE (2026-07-14, ROOT-CAUSED): a MULTI-ARM `match` (≥4 arms → lowered to a `br_table`) used
;; as an OPERAND of a binary op (here `String.concat`) discards the OTHER operand. `cdz check` is CLEAN;
;; the wasm is valid but computes the wrong value.
;;
;;   go(4) should be "bb" (byte-len 2) but returns "b" (byte-len 1) — the recursive left operand
;;   `(go (/ n 3))` is dropped; only the right `(d (% n 3))` survives.
;;
;; ROOT CAUSE (from the emitted WAT of `go`): in `(String.concat (go …) (d …))`, the recursive `call`
;; pushes its handle, then the digit `match` lowers to nested blocks + a `br_table` whose every arm ends
;; `br N (;@1;)` — branching to the FUNCTION-RESULT label, i.e. OUT of the whole function, PAST the
;; `bytes-concat`. So the concat never runs and the left operand is discarded. A `br_table` match arm's
;; branch target is the function/enclosing-result label instead of the match block's own end, so a match
;; in operand position (not tail position) escapes its context.
;;
;; THRESHOLD: exactly ≥4 match arms (which triggers the `br_table` lowering); a 2- or 3-arm match (an
;; if/probe chain) as the same operand works. So it is the br_table-lowered-match-in-operand-position.
;;
;; SHARPER: the SIBLING operand must be a RECURSIVE CALL for the drop to bite. Verified on integer `+`
;; too (not String-specific): `(+ (go (/ n 3)) (d (% n 3)))` with a 4-arm `d` drops `go`'s result; but
;; `(+ param (d m))` and `(+ (3-arm-match k) (d k))` (a non-recursive sibling) both compute correctly.
;; So: a br_table-lowered match in operand position + a recursive-call sibling → the sibling is dropped
;; (the match's arm `br` targets the enclosing-result label, escaping past the op that consumes both).
;; CONTROL (correct): drop a match arm to 3 (if-chain lowering), or make the sibling non-recursive.
(do
  (def (d (: v Int64)) (match v (0 "a") (1 "b") (2 "c") (_ "?")))
  (def (go (: n Int64)) (if (< n 3) (d n) (String.concat (go (/ n 3)) (d (% n 3)))))
  (def (main) (String.byte-len (go 4)))
  (export main))
