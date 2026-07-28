;; PIN-PENDING-LAND: add to spec/semantics/06-numeric-model.sexp once v-inference's small-operand
;; shift-left width-generic fold (MR b2197d097) lands. On trunk ecf272d2d the case still DECLINES
;; CDZ0304 (gate-probed: (<< (: 1 UInt64) 63) rejects on wasm+rust+rust-async), so it would gate-FAIL
;; now — HELD.
;;
;; Follow-up to the wide-operand shift/bitwise slice (edc5e15bf, pinned as 2ccf5129d). That slice only
;; reached the wide block when an OPERAND exceeded i64; this case has SMALL operands (both fit i64) but a
;; RESULT that overflows i64 while fitting UInt64 — checked_shl_i64 overflow-checked against Int64
;; regardless of the solved width, so it spuriously declined. b2197d097 extracts fold_shift_bitwise_at_width
;; and folds shift/bitwise over the SOLVED width for BOTH wide-operand and small-operand-wide-result cases
;; (unsigned-only; signed >> stays on i64 to sign-extend). (<< (: 1 UInt64) 63) now folds to 2^63.
;;
;; v-inference CONFIRMED (note 15521) the u8/u16/u32 << siblings were NEVER broken (2^7/2^15/2^31 all fit
;; i64, so checked_shl_i64 didn't spuriously overflow) — ONLY UInt64 was affected. The u32 1<<31 case below
;; is an ALREADY-CORRECT width-generic guard (locks the uniform fold), not a bug fix.
;;
;; ON LAND (b2197d097 on trunk): rebuild cdz, gate-probe both cases PASS on wasm+rust+rust-async, insert
;; after the "shift count at or beyond the width" case, baseline (2 pass) x3, verify titles-agree/0-dup/
;; 0-omission + gate --check all 3 + roundtrip, commit + MR, notify v-inference.

(case "a constant shift-left of a small UInt64 operand whose result overflows i64 folds over the solved width"
  (doc    "`(<< (: 1 UInt64) (: 63 UInt64))` — both operands fit i64, but the result 2^63 overflows i64 while
           fitting UInt64. The old checked_shl_i64 overflow-checked against Int64 regardless of the solved
           width, so it spuriously declined CDZ0304. The width-generic fold (b2197d097) folds over the SOLVED
           width: 1 << 63 = 9223372036854775808. Expected: 9223372036854775808.")
  (input  (do (def (main) (<< (: 1 UInt64) (: 63 UInt64))) (export main)))
  (output (: 9223372036854775808 UInt64)))

(case "a constant shift-left of a small UInt32 operand into its high bit folds (width-generic guard)"
  (doc    "The already-correct width-generic guard: `(<< (: 1 UInt32) (: 31 UInt32))` = 2^31 = 2147483648.
           This ALWAYS folded (2^31 fits i64, so the old checked_shl_i64 never spuriously overflowed) — pinned
           to LOCK the width-generic fold's uniform behavior across widths, not because it was ever broken.")
  (input  (do (def (main) (<< (: 1 UInt32) (: 31 UInt32))) (export main)))
  (output (: 2147483648 UInt32)))
