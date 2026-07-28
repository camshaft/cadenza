; FINDING (breaker, 2026-07-25): a handler-arm do-def referenced in BOTH resume slots
; (value AND next-state) is CDZ0101 "unbound name" in a LIVE (non-folded) handler — a
; FALSE REJECT. The single-use faces and the let-form all work, so this is the multi-use
; residue of the #21 do->let normalization (v-effects e49c698a1 fixed the PERFORM-arg path;
; the RESUME-arg path handles single-use but loses the binder when one do-def feeds both args).
;
; MATRIX (all with a runtime operand so the handler stays live; const-foldable versions PASS):
;   x (do (def d ...) (resume d d))            - ONE def, BOTH slots        -> CDZ0101 unbound d
;   x heap twin: (def s2 (List.push s v)) (resume (List.len s2) s2)         -> CDZ0101 unbound s2
;   ok (do (def d ...) (resume d s))           - value slot only            -> compiles
;   ok (do (def d ...) (resume v d))           - state slot only            -> compiles
;   ok (do (def d ...) (def e ...) (resume d e)) - TWO defs, one per slot   -> compiles
;   ok (let ((s2 ...)) (resume (List.len s2) s2)) - LET, both slots         -> compiles + runs
;   ok const-foldable (resume d d)             - folds before the arm lowers -> passes
;
; IMPACT: the natural accumulator-arm shape - compute the new state ONCE, resume both the
; derived value and the state from it ((def s2 (List.push s v)) (resume (List.len s2) s2)) -
; false-rejects; authors must either duplicate the computation or use let. Same fix
; neighborhood as #21 (reduce_handle normalization), so likely a small patch for v-effects.
;
; Repro (expect 33: note-values 1,2 -> (1*10+2)=12... see graded case; actual: CDZ0101):
(case "a do-def shared across BOTH resume slots lowers in a live handler (FALSE-REJECT repro)"
  (input (do
        (effect L (op note (-> Int64 Int64)))
        (def (main (: n Int64))
          (handle L (list)
            ((note (v) s (do (def s2 (List.push s v)) (resume (List.len s2) s2))))
            (+ (* (L.note n) 10) (L.note 20))))
        (export main)))
  (call main (: 5 Int64)) (output (: 12 Int64)))
