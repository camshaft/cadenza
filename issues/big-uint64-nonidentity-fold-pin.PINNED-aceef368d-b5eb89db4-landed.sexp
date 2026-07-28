;; PIN-PENDING-LAND: add to spec/semantics/06-numeric-model.sexp once v-inference's big-u64
;; NON-identity constant fold follow-up (MR b5eb89db4) lands. On trunk cb4bf24fe the fold-correct
;; ops still REJECT CDZ0304 (verified: (/ big-u64 2) → CDZ0304), so the 4 fold-correct cases
;; would gate-FAIL now — HELD. The 2 negative cases (real overflow / div-by-zero) already
;; decline today but are pinned together for one coherent set.
;;
;; Follow-up to the IDENTITY set (high-uint64-constant-identity-fold, landed a9a18dfb1) — makes
;; NON-identity big-u64 constant ops (/ - % and big+) fold over exact IntValue (not i64), so a
;; UInt64 operand > Int64.max folds to the true result instead of spuriously declining.
;; Order-independent from the identity set (identities stay identities).
;;
;; ON LAND: gate all 6 on wasm+rust+rust-async, baseline (4 pass + 2 todo for the negatives),
;; verify titles-agree/0-dup/0-omission + gate --check, commit + MR, notify v-inference.

(case "a constant division over a high UInt64 operand folds exactly to the quotient"
  (doc    "`(/ (: 18446744073709551614 UInt64) (: 2 UInt64))` — the dividend is (UInt64.max-1), above
           Int64.max. The fold evaluates over exact IntValue (not i64), so it folds to the true quotient
           9223372036854775807 rather than spuriously declining. Expected: 9223372036854775807.")
  (input  (do (def (main) (/ (: 18446744073709551614 UInt64) (: 2 UInt64))) (export main)))
  (output (: 9223372036854775807 UInt64)))

(case "a constant subtraction over a high UInt64 operand folds to the difference"
  (doc    "`(- (: 18446744073709551615 UInt64) (: 1 UInt64))` — UInt64.max - 1 folds over exact IntValue to
           18446744073709551614, no i64 needed for the high operand.")
  (input  (do (def (main) (- (: 18446744073709551615 UInt64) (: 1 UInt64))) (export main)))
  (output (: 18446744073709551614 UInt64)))

(case "a constant modulo over a high UInt64 operand folds to the remainder"
  (doc    "`(% (: 18446744073709551615 UInt64) (: 10 UInt64))` — UInt64.max mod 10 folds to 5 over exact
           IntValue (2^64-1 = ...615, last digit 5).")
  (input  (do (def (main) (% (: 18446744073709551615 UInt64) (: 10 UInt64))) (export main)))
  (output (: 5 UInt64)))

(case "a constant add of two high UInt64 operands within range folds to the sum"
  (doc    "`(+ (: 9223372036854775808 UInt64) (: 5 UInt64))` — 2^63 + 5 = 9223372036854775813, still within
           UInt64 range (< 2^64), folds over exact IntValue (the left operand is above Int64.max).")
  (input  (do (def (main) (+ (: 9223372036854775808 UInt64) (: 5 UInt64))) (export main)))
  (output (: 9223372036854775813 UInt64)))

(case "a constant MUL over a high UInt64 operand that overflows still rejects"
  (doc    "The boundary guard: `(* (: 18446744073709551615 UInt64) (: 2 UInt64))` — UInt64.max * 2 exceeds
           2^64-1, a GENUINE unsigned overflow → must still reject CDZ0304, so the big-u64 fold does not
           silently wrap. Only in-range results fold; a real overflow is a compile-time reject.")
  (input  (do (def (main) (* (: 18446744073709551615 UInt64) (: 2 UInt64))) (export main)))
  (error  CDZ0304))

(case "a constant division by zero over a high UInt64 operand still rejects"
  (doc    "The div-by-zero boundary: `(/ (: 18446744073709551615 UInt64) (: 0 UInt64))` — a constant
           divide-by-zero must still reject CDZ0304 (a trap the fold must not swallow), even with the
           big-u64 exact-IntValue fold path.")
  (input  (do (def (main) (/ (: 18446744073709551615 UInt64) (: 0 UInt64))) (export main)))
  (error  CDZ0304))
