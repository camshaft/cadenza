;; PIN-PENDING-LAND: add to spec/semantics/02-binding-and-control.sexp once v-value-facts'
;; UInt64-ceiling soundness FIX b4ec1016f lands on trunk. On current trunk f45c7834a the
;; miscompile is LIVE — I reproduced it: f(2^63) returns 2 (WRONG), correct is 1. Adding the
;; case now would gate-fail (case expects 1, trunk yields 2), so it is HELD.
;;
;; Root cause (v-value-facts): refine_from_comparison seeded the interval from a hardcoded
;; i64::MAX instead of resolved_int_bounds (UInt64 hi = None). A lower-bound refinement
;; (> x 8) wrongly fabricated an i64::MAX ceiling, so (> x 9223372036854775807) folded to
;; FALSE. The fold operand i64::MAX IS i64-representable so the fold CAN fire → load-bearing
;; miscompile. Only bit UInt64 (types whose max exceeds i64::MAX); UInt32/Int* always fine.
;;
;; This is the THIRD slice-2 GAP-A witness; the UInt32 same/implied/soundness-twin pin
;; (6303c9f03, pending) already covers the other two shapes and is unaffected by this fix.

(case "an UNSIGNED lower-bound refinement must not fabricate an i64::MAX ceiling for a UInt64 above it"
  (doc    "The UInt64-ceiling soundness pin for value-facts GAP-A (rcdzc b4ec1016f). A lower-bound
           refinement `(> x 8)` must NOT conclude `x <= i64::MAX` — a UInt64 ranges past i64::MAX. So the
           nested `(> x 9223372036854775807)` (i.e. `> i64::MAX`) must stay LIVE, not fold to false. The
           fold operand i64::MAX is itself i64-representable, so a buggy refinement CAN fire the fold — the
           load-bearing case. At x = 2^63 = 9223372036854775808 (a valid UInt64 one past i64::MAX) the inner
           test is TRUE → 1; a miscompiling fold would yield 2. Root fix seeds the interval from
           resolved_int_bounds (UInt64 hi = None), not a hardcoded i64::MAX.")
  (input  (do (def (f (: x UInt64)) (if (> x 8) (if (> x 9223372036854775807) 1 2) 0)) (export f)))
  (call   f (: 9223372036854775808 UInt64))
  (output (: 1 Int64))
  ; control: a genuinely small x takes neither refined path → 0
  (call   f (: 5 UInt64))
  (output (: 0 Int64)))
