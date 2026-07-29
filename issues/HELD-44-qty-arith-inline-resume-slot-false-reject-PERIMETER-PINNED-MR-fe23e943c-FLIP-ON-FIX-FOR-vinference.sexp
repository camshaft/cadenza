; FINDING #44 (breaker): a Qty+Qty arithmetic expression INLINE in a handler's resume slot
; (either the VALUE slot or the NEXT-STATE slot) is falsely typed as the ERASED inner scalar
; (Int64) and rejected — while the semantically identical expression bound via an arm-local
; `def` first type-checks AND runs correctly. False reject + workaround inconsistency.
;
;   (handle Acc (Qty.of a meter)
;     ((step (_u) s (resume s (+ s s))))          ; REJECTS: "next-state of type Int64 but state
;                                                 ;  type is (Qty Int64 meter)" — but s IS the Qty
;     ...)
;   ((step (_u) s (resume (+ s s) s)))            ; REJECTS: "resumes with a value of type Int64
;                                                 ;  but the operation's result type is (Qty ...)"
;   ((step (_u) s (do (def t (+ s s)) (resume t s))))  ; ACCEPTS and runs → 42 at a=21
;
; The state binder `s` seeded with a Qty seems to lose its Qty type exactly when consumed by an
; arithmetic op INSIDE the resume-slot expression — the checker types (+ s s) at the erased inner
; scalar (Int64) and then the slot check compares Int64 vs (Qty ...). Control: (+ q q) over a Qty
; in a PLAIN fn types fine; (resume s s) pass-through is fine; Qty.value/re-wrap in the slot is fine.
; Lane guess: v-inference (handler-arm slot typing runs before/without the Qty layer's op typing?)
; or the effects fold's arm typing. Probed on trunk fc2b91731.
;
; Witness 1 — the accepted arm-local form runs (pins the SEMANTICS the inline form must match):
(case "Qty arithmetic on the handler state binder via an arm-local def threads and runs"
  (input  (do
            (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main (: a Int64))
              (handle Acc (Qty.of a (Unit.base #"meter"))
                ((step (_u) s (do (def t (+ s s)) (resume t s))))
                (Qty.value (Acc.step))))
            (export main)))
  (call   main (: 21 Int64))
  (output (: 42 Int64)))

; Witness 2 — value+re-wrap state advance runs (the heavier workaround):
(case "a Qty handler state advances via Qty.value / re-wrap in the next-state slot"
  (input  (do
            (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main (: a Int64))
              (handle Acc (Qty.of a (Unit.base #"meter"))
                ((step (_u) s (resume s (Qty.of (* (Qty.value s) 2) (Unit.base #"meter")))))
                (Qty.value (+ (Acc.step) (Acc.step)))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 15 Int64)))

; Witness 3 (currently REJECTS — flips when the inline slot typing is fixed; same semantics as W1):
; (case "Qty arithmetic INLINE in the next-state slot types at the Qty, not the erased scalar"
;   (input  (do
;             (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
;             (def (main (: a Int64))
;               (handle Acc (Qty.of a (Unit.base #"meter"))
;                 ((step (_u) s (resume s (+ s s))))
;                 (Qty.value (Acc.step))))
;             (export main)))
;   (call   main (: 21 Int64))
;   (output (: 21 Int64)))

;; ------------------------------------------------------------------------------------------
;; TRIAGED-CONFIRMED (corpus-bugfix, trunk fc2b91731, both backends): arm-local-def form runs 42; inline
;; (resume s (+ s s)) rejects CDZ0201 (types (+ s s) at erased Int64 vs the (Qty Int64 meter) state type).
;; Control: (+ q q) on a Qty param in a plain fn is FINE — it's specifically the resume-SLOT expression
;; context. False reject + workaround inconsistency; NOT a soundness hole; ergonomics blocker for the
;; v-cad/notebook @param Qty-stateful-handler pattern.
;; PERIMETER PINNED (MR fe23e943c): the 2 working forms (arm-local-def 42 + Qty.value/re-wrap 15) are in
;; 14-effects. The INLINE reject form is HELD as a FLIP-PIN: when v-inference fixes the slot typing to keep
;; the state binder's Qty type through inline arithmetic, (resume s (+ s s)) → runs (42) and I flip this
;; from reject to a value pin. OWNER: v-inference (handler-arm slot typing vs the Qty layer op typing order).
;; ON FIX: gate the inline form x3 → 42; pin into 14-effects beside the perimeter; baseline x3.

(case "Qty arithmetic INLINE in a handler resume slot keeps the state binder's Qty type (was a false Int64 reject)"
  (input  (do
            (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main (: a Int64))
              (handle Acc (Qty.of a (Unit.base #"meter"))
                ((step (_u) s (resume s (+ s s))))
                (Qty.value (Acc.step))))
            (export main)))
  (call   main (: 21 Int64))
  (output (: 42 Int64)))
