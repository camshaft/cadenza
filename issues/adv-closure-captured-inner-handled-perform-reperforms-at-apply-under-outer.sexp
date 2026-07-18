; MISCOMPILE (silent wrong value, BOTH backends agree -> Core-level lowering).
; A closure built inside an INNER handle, capturing a let-bound perform result (`base`), escapes to the
; OUTER handler's scope. The captured `base` is NOT the inner-handled VALUE: each APPLICATION of the
; closure RE-performs the tick at the apply site, homed by the OUTER handler — wrong value, wrong home,
; and outer state advances per call. Escape analysis correctly gives the shape a home (the outer handler),
; so nothing rejects; lowering then substitutes the perform INTO the closure body instead of memoizing the
; captured value at construction time. Family: v-effects' parked re-perform/operand-memo miscompiles
; (host-arg re-performs; @param fn-arg-reperform) — this face needs NO host and NO @param, just two
; nested handles of one effect. Controls that ISOLATE the trigger: capturing a PLAIN escaped value
; (no closure) is correct; applying the closure INSIDE the inner handle is CDZ0401-rejected (adjacent
; over-reject, separately filed); it is exactly closure-capture + escape + apply-under-outer that breaks.

(case "a closure capturing an inner-handled perform result is applied after the inner handle exits — the capture must be the VALUE, not a re-perform"
  (doc    "`base` is let-bound to `(Ctr.tick)` under the INNER handle (seed 50), so base=50 and the closure
           is (fn (x) (+ x 50)). Applied twice under the OUTER handler (seed 5), the result must be
           50*100 + 50 = 5050. MISCOMPILES to 506 = 5*100 + 6 on wasm AND rust: each apply re-performs
           the tick under the outer handler (5, then 6) — the capture was compiled as the perform
           EXPRESSION, not its value.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main)
              (handle Ctr 5
                ((tick (u) s (resume s (+ s 1))))
                (let ((f (handle Ctr 50
                           ((tick (u) s (resume s (+ s 1))))
                           (let ((base (Ctr.tick)))
                             (fn ((: x Int64)) (+ x base))))))
                  (+ (* 100 (f 0)) (f 0)))))
            (export main)))
  (output (: 5050 Int64)))

(case "single-apply face: the captured inner tick plus a const argument"
  (doc    "Same shape, one application with const argument 3: base=50 under the inner handle, f(3) must be
           53. MISCOMPILES to 8 = 3 + 5 (the re-performed tick under the OUTER handler's seed). The
           smallest wrong-value witness of the family.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main)
              (handle Ctr 5
                ((tick (u) s (resume s (+ s 1))))
                (let ((f (handle Ctr 50
                           ((tick (u) s (resume s (+ s 1))))
                           (let ((base (Ctr.tick)))
                             (fn ((: x Int64)) (+ x base))))))
                  (f 3))))
            (export main)))
  (output (: 53 Int64)))

(case "control: a PLAIN value escaping the inner handle is correct — the closure capture is the trigger"
  (doc    "The no-closure control: `base` (the inner tick, 50) escapes as a plain Int64 and is added to an
           outer tick (5) — 55, and this PASSES today on both backends. Pins the boundary: value escape is
           sound; only routing the capture through a CLOSURE breaks it.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main)
              (handle Ctr 5
                ((tick (u) s (resume s (+ s 1))))
                (let ((base (handle Ctr 50
                              ((tick (u) s (resume s (+ s 1))))
                              (Ctr.tick))))
                  (+ base (Ctr.tick)))))
            (export main)))
  (output (: 55 Int64)))

; ---
; ROUTED to v-effects (corpus-bugfix 2026-07-18, VERIFIED both backends: f(3)=8, expected 53). SILENT
; MISCOMPILE: a closure capturing an INNER-handled perform result re-performs at each apply, re-homed by
; the OUTER handler (wrong value + wrong home + outer state advances per call). The captured `base` is
; compiled as the perform EXPRESSION, not its construction-time VALUE. Controls: plain-value escape CORRECT
; (55); apply INSIDE inner handle = CDZ0401-over-reject (the adjacent parked escape-analysis gap). Family:
; v-effects' parked host-arg-reperform / operand-memo / @param fn-arg-reperform — minimal no-host no-@param
; face, likely SAME ROOT (capture memoization: a captured perform-result must be memoized to its
; construction-time value). MISCOMPILE (not a reject-gap) -> higher priority than the parked faces. Not spawning.
