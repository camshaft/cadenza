; adv-60 (breaker, 2026-08-02, LOW-MED diagnostics — reject-either-way, but WRONG CODE CLASS and
; span-less; gate-visible/corpus-pinnable unlike adv-58/59): a CONSTANT rational whose truncate
; exceeds Int64 rejects `[CDZ0302]: integer literal does not fit its width` with NO node/span —
; but no literal in the program is out of width (9223372036854775807 fits Int64 exactly). The
; overflow is produced by the CONST FOLD (3·MAX as an exact rational), then the folded BigInt
; integer part is apparently re-injected as an Int64 literal and trips the backend width check.
; expected: CDZ0304 ("constant <op> traps ..."), the code every other compile-provable constant
; overflow gets — e.g. (+ Int64.max 1) → CDZ0304 with a node. Bracket matrix:
;   runtime shape (corpus pin, 06-numeric:247): traps unreachable at run time — CORRECT, PASSES.
;   const in-range truncate: 9223372036854775807 — CORRECT.
;   const Int64 add overflow: CDZ0304 w/ node — CORRECT (the class this case should join).
;   const truncate overflow: CDZ0302, span-less — WRONG CLASS + no location.
; note: `cdz check` does NOT surface it at all (only a bogus arity error from the same fold path?)
; — the reject appears only at emit, so the diagnostic never reaches the IDE surface either.
(case "adv-60 a CONST rational truncate whose integer part exceeds Int64 is CDZ0304 (const-op-traps), not a span-less CDZ0302"
  (input  (Rational.truncate (* (Rational.of 9223372036854775807 1) (Rational.of 3 1))))
  (error  CDZ0304))

; --- BROADENED (v-core-opt, MR c657cb0de): the fix covers ALL FOUR rounding ops, not just truncate ---
; floor/ceil/round had the identical divmod→ConstInt-without-fit-check bug. Pin all 4 on land (each
; flips todo→pass when c657cb0de lands; today all reject the wrong span-less CDZ0302).
(case "adv-60 a CONST rational floor whose integer part exceeds Int64 is CDZ0304, not span-less CDZ0302"
  (input  (Rational.floor (* (Rational.of 9223372036854775807 1) (Rational.of 3 1))))
  (error  CDZ0304))
(case "adv-60 a CONST rational ceil whose integer part exceeds Int64 is CDZ0304, not span-less CDZ0302"
  (input  (Rational.ceil (* (Rational.of 9223372036854775807 1) (Rational.of 3 1))))
  (error  CDZ0304))
(case "adv-60 a CONST rational round whose integer part exceeds Int64 is CDZ0304, not span-less CDZ0302"
  (input  (Rational.round (* (Rational.of 9223372036854775807 1) (Rational.of 3 1))))
  (error  CDZ0304))
