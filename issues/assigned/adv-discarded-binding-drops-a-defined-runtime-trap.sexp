; ADVERSARIAL FINDING (producer, iter-393, 2026-07-14) — 🔴 MISCOMPILE (dropped trap) / SPEC QUESTION: a
; 0-use `let` binding (or a discarded do-statement) whose initializer has a DEFINED RUNTIME TRAP — a
; divide-by-zero, a zero-remainder, or a zero-denominator `Rational.of` — is silently DROPPED by DCE, so the
; trap never fires and the program returns a value where it should have trapped. The IDENTICAL initializer in
; a USED position DOES trap. This is the trap-preservation twin of the dead-binding elimination: DCE removes
; the binding (correctly warned CDZ0306 "unused binding") but ALSO removes its side-effecting/ trapping
; initializer, which is only sound if `let` is non-strict (lazy). If `let` is STRICT (the initializer is
; evaluated for its effect/trap even when its value is unused), this is a MISCOMPILE.
;
; This finding is a fresh, minimal, multi-op witness of the class the spec owner was already asked to rule on
; (is `let` strict?). 057e76bb pinned that these ops trap in a USED position (div/rem/rational-of zero
; divisor) — but the DISCARDED position drops the very same trap.
;
; REPRODUCER (returns 1 — WRONG if `let` is strict; should TRAP "integer divide by zero"):
;   (do (def (main (: d Int64)) (let ((q (/ 100 d))) 1))
;       (export main))
;   run with d = 0  →  1   (the `(/ 100 0)` div-by-zero trap is DROPPED because `q` is unused; DCE removed the
;                           whole binding incl. its trapping init)
;
; ISOLATION (the trap is dropped ONLY when the trapping expression's value is UNUSED; deterministic 3×):
;   let q = (/ 100 d) in 1        , d=0   → 🔴 1     (div-by-zero trap DROPPED)
;   let q = (/ 100 d) in q        , d=0   → traps    [OK — USED position, the trap fires]
;   let q = (% 100 d) in 1        , d=0   → 🔴 1     (rem-by-zero trap DROPPED)
;   let q = (Rational.of 1 d) in 1, d=0   → 🔴 1     (zero-DENOMINATOR trap DROPPED)
;   (Rational.of 1 d) in return   , d=0   → traps    [OK — returned/used, the zero-denominator trap fires]
;   (do (/ 100 d) 1)             , d=0   → 🔴 1     (a discarded DO-STATEMENT also drops the trap)
;   let q = (/ 100 d) in 1        , d=5   → 1        [OK — no trap defined, correctly 1]
;   → so the trap is dropped exactly when the trapping op's result is discarded (0-use let binding or a
;     non-final do-statement). div, rem, and Rational.of (zero denominator) all exhibit it; the USED position
;     of each traps correctly. A CDZ0306 "unused binding" warning fires, then DCE drops the binding AND its
;     trapping init.
;
; ROOT CAUSE (hypothesis, lower.rs DCE — `should_keep_binding`): a 0-use binding is eliminated as dead. The
; elimination is value-correct (the value is never read) but drops the initializer's DEFINED TRAP. The fix
; (if `let` is strict) is to KEEP a 0-use binding whose init is NOT `is_trap_free` (evaluate it for its trap,
; discard the value) — the same trap-freedom predicate LICM/select use to gate speculative evaluation. Affects
; BOTH backends (the DCE is in shared lowering). This is the SPEC-OWNER-flagged dead-binding-drops-trap
; question with div/rem/Rational.of witnesses.
;
; SEVERITY: 🔴 MISCOMPILE (IF `let` is strict) — a program that must trap silently returns a value; no
; diagnostic beyond the unused-binding warning. Alternatively, if DCE-of-partial-ops on discarded values is
; INTENDED (lazy `let`), this is CORRECT and the case documents the semantics. Either way it needs a
; spec ruling; graded as Fail (returns 1 where a trap is expected, under strict-let semantics — the more
; common ML/eager default). Reachable from any "compute-and-discard for its effect" idiom (a validation call
; whose result is ignored, an assertion-shaped `(do (checked-op …) result)`).

(case "a discarded binding whose initializer has a defined trap still traps"
  (doc    "`(let ((q (/ 100 d))) 1)` with d=0 — the binding `q` initialized to `(/ 100 0)` is never used, so
           the body returns the constant 1. Under strict-`let` semantics the initializer `(/ 100 0)` is
           evaluated for its effect and must TRAP (integer divide by zero). Instead the program returns 1:
           DCE eliminated the 0-use binding AND its trapping initializer (a CDZ0306 unused-binding warning
           fires first). The IDENTICAL `(/ 100 d)` in the body position `(let ((q (/ 100 d))) q)` traps
           correctly at d=0 — so the trap is dropped exactly when the value is discarded. Same for a discarded
           `(% 100 0)`, a zero-denominator `(Rational.of 1 0)`, and a non-final do-statement `(do (/ 100 d) 1)`
           — while each traps in a used position (057e76bb pins the used-position traps). Fix (if `let` is
           strict): keep a 0-use binding whose init is not `is_trap_free`, evaluating it for its trap. This is
           the spec-owner-flagged dead-binding-drops-trap question. Expected (strict let): trap 'integer
           divide by zero'.")
  (input  (do
            (def (main (: d Int64)) (let ((q (/ 100 d))) 1))
            (export main)))
  (trap "integer divide by zero"))
