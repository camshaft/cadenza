;; HELD PIN (corpus-bugfix, 2026-07-25) — do NOT land until v-effects fixes the false-reject.
;; Origin: breaker FINDING (inbox issue 000000016861). CONFIRMED on trunk bce8ba646 (reproduced +
;; sharpened by corpus-bugfix): a handler-arm do-def referenced in BOTH resume slots (value AND
;; next-state) is CDZ0101 "unbound" in a LIVE (runtime-operand) handler — a FALSE REJECT. The multi-use
;; residue of #21's do->let normalization (v-effects e49c698a1 fixed the perform-arg + single-use
;; resume-arg paths; loses the binder when ONE do-def feeds BOTH resume args).
;;
;; DISCRIMINATOR (corpus-bugfix verified on trunk — sharpened past breaker's matrix):
;;   • (do (def d …) (resume d d))  [scalar, ONE def BOTH slots]      → CDZ0101 unbound  ← BUG
;;   • (do (def s2 (List.push s v)) (resume (List.len s2) s2)) [heap]  → CDZ0101 unbound  ← BUG (repro)
;;   • (let ((s2 …)) (resume (List.len s2) s2))  [LET, both slots]     → COMPILES + runs (→ 12) ✓ oracle
;;   • (do (def d …) (resume d s))  [do-def value slot only]          → compiles ✓
;;   • (do (def d …)(def e …) (resume d s))  [2 defs, resume refs ONE] → compiles ✓
;;   ⇒ the trigger is SPECIFICALLY a single do-def bound name referenced in BOTH resume args; a do-def
;;     referenced in only one slot (even with a sibling def) compiles. So the normalization loses the
;;     binder when it's multiply-referenced across the two resume operands.
;;   • (do (def d (+ v 1)) (resume (+ d d) s))  [dual-ref WITHIN one operand]  → COMPILES (breaker #24
;;     perimeter, corpus-bugfix-verified → 162). So multi-reference WITHIN a slot is fine; the break is
;;     STRICTLY CROSS-SLOT — a shared do-def spanning the value-arg AND the state-arg. Tightens the fix
;;     site to how the two resume operands are lowered as SEPARATE scopes/copies (the shared binder is
;;     lost across that split, not within a single operand's scope).
;; ORACLE: the LET-form of the exact repro compiles + runs → 12 on wasm AND rust; the do-form MUST match
;;   (→ 12) once fixed. Scalar and heap twins both repro. Likely a small v-effects reduce_handle patch.
;; OWNER: v-effects (same normalization neighborhood as #21 e49c698a1).
;; ON LAND (v-effects' fix on trunk): rebuild cdz; gate the do-form case x3 (→ 12) + keep the let-form
;;   companion; pin BOTH beside the do-def-in-perform-arg pins in 14-effects (the #21 family); baseline
;;   x3; roundtrip + silent-omission + --check; MR; notify v-effects + breaker (multi-use resume residue
;;   of #21 closed).

(case "a do-def shared across BOTH resume slots lowers in a live handler (FALSE-REJECT repro)"
  (input (do
        (effect L (op note (-> Int64 Int64)))
        (def (main (: n Int64))
          (handle L (list)
            ((note (v) s (do (def s2 (List.push s v)) (resume (List.len s2) s2))))
            (+ (* (L.note n) 10) (L.note 20))))
        (export main)))
  (call main (: 5 Int64)) (output (: 12 Int64)))

(case "the let-form of the dual-resume-slot arm compiles and runs (the always-worked oracle)"
  (input (do
        (effect L (op note (-> Int64 Int64)))
        (def (main (: n Int64))
          (handle L (list)
            ((note (v) s (let ((s2 (List.push s v))) (resume (List.len s2) s2))))
            (+ (* (L.note n) 10) (L.note 20))))
        (export main)))
  (call main (: 5 Int64)) (output (: 12 Int64)))
