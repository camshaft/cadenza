;; PIN-PENDING-LAND: add to spec/semantics/06-numeric-model.sexp once v-inference's high-UInt64
;; constant-identity fold fix (MR 819f5c2e3) lands on trunk. On current trunk c6f00f531 the
;; bug is LIVE — verified: (+ (: 18446744073709551615 UInt64) (: 0 UInt64)) spuriously rejects
;; CDZ0304. Adding the 3 fold-correct cases now would gate-FAIL (they expect a value, trunk
;; rejects), so the whole set is HELD.
;;
;; Root (v-inference): lower_arith folded both-constant ops over i64, so a UInt64 operand >= 2^63
;; (e.g. UInt64.max) had no i64 and got wrongly rejected. Fix = width-agnostic algebraic
;; identities (x+0 / x-0 / x*1 / x*0) fold at any width. --lib test
;; a_constant_algebraic_identity_over_a_high_uint64_operand_folds_not_declines guards it
;; (not gate-visible); this corpus pin is the load-bearing fleet-wide guard.
;;
;; ON LAND: gate all 4 on wasm+rust+rust-async, baseline (3 pass + 1 todo for the negative),
;; verify titles-agree/0-dup/0-omission + gate --check, commit + MR, notify v-inference.

(case "a constant algebraic identity over a high UInt64 operand folds to the operand, not a spurious reject"
  (doc    "`(+ (: 18446744073709551615 UInt64) (: 0 UInt64))` — a constant `x + 0` over a UInt64 operand
           at UInt64.max (2^64-1, above Int64.max). The i64-only constant fold had no i64 for the operand
           and REJECTED it CDZ0304 (constant operand does not fit the integer width) — a spurious decline of
           valid unsigned arithmetic. The width-agnostic identity `x + 0 = x` folds to the operand at any
           width. Expected: 18446744073709551615.")
  (input  (do (def (main) (+ (: 18446744073709551615 UInt64) (: 0 UInt64))) (export main)))
  (output (: 18446744073709551615 UInt64)))

(case "a constant MUL-by-1 identity over a high UInt64 operand folds to the operand"
  (doc    "The `x * 1 = x` twin of the add-zero identity above, at UInt64.max — folds width-agnostically
           to the operand rather than spuriously rejecting on the missing i64.")
  (input  (do (def (main) (* (: 18446744073709551615 UInt64) (: 1 UInt64))) (export main)))
  (output (: 18446744073709551615 UInt64)))

(case "a constant MUL-by-0 identity over a high UInt64 operand folds to zero"
  (doc    "The `x * 0 = 0` twin at UInt64.max — folds to 0 width-agnostically (the annihilator identity),
           no i64 needed for the high operand.")
  (input  (do (def (main) (* (: 18446744073709551615 UInt64) (: 0 UInt64))) (export main)))
  (output (: 0 UInt64)))

(case "a NON-identity constant add over a high UInt64 operand still rejects the overflow"
  (doc    "The boundary guard: `(+ (: 18446744073709551615 UInt64) (: 1 UInt64))` — UInt64.max + 1 is a
           GENUINE unsigned overflow and MUST still reject CDZ0304, so a future over-eager u64 constant
           fold does not silently wrap/miscompile. Only the width-agnostic identities (0/1 operand) fold;
           a real constant overflow is still a compile-time reject.")
  (input  (do (def (main) (+ (: 18446744073709551615 UInt64) (: 1 UInt64))) (export main)))
  (error  CDZ0304))
