; breaker probe N — the fused-clone seam × EFFECTS: a match on a CALL result (fusion candidate)
; whose arms PERFORM to a stateful handler. If fusion clones the arm into each branch of the
; callee's `if`, the perform must not be duplicated in a way that double-steps the state, and
; state order must survive the transform.
; Hand-derived: handler Fresh seeded 0, arm resumes s, next-state s+1.
;   body: (match (mk k) ((Hi h) (+ (* 10 h) (Fresh.next))) ((Lo w) (+ (* 100 w) (Fresh.next))))
;   then + (Fresh.next) at the end.
;   k=7: mk→Hi 7 → arm: 70 + next(reads 0, →1) = 70; then final next reads 1 → 70+1... total (+ arm final)
;   = 70 + 0 + 1 = 71. Wait: arm = 70 + 0 = 70; total = 70 + 1 = 71.
;   k=2: mk→Lo 2 → 200 + 0 = 200; total = 200 + 1 = 201.
;   Exactly ONE perform runs in the match (the taken arm) + one after → state ends at 2 either way,
;   but the VALUE encodes the order. A fusion that duplicated the arm perform per branch would still
;   run once dynamically (branches are exclusive) — the hazard is the handler-frame threading through
;   the cloned arms: a clone that re-seeded or lost the state gives 70+0+0 or wrong digits.

(case "a stateful perform inside the arm of a fused match on a call result threads state once"
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (type Sz (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (def (main (: k Int64))
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (+ (match (mk k)
                     ((Hi h) (+ (* 10 h) (Fresh.next)))
                     ((Lo w) (+ (* 100 w) (Fresh.next))))
                   (Fresh.next))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 71 Int64))
  (call   main (: 2 Int64)) (output (: 201 Int64)))
