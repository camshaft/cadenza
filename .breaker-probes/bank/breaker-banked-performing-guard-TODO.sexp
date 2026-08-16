; breaker probe R — a PERFORMING guard: the guard expression itself performs to a stateful handler.
; Guards are tried top-to-bottom; each TRIED guard's perform must fire exactly once per try, in arm
; order, and an arm whose PATTERN fails must NOT run its guard (no perform). The value encodes the
; state history.
; Hand-derived: handler Ctr seeded 0, arm resumes s, next-state s+1.
;   match on (mk k): mk 7 → (Hi 7).
;   arm1 pattern (Hi h) MATCHES → guard (> (+ h (Ctr.next)) 10): next reads 0 (state→1) → 7+0=7 > 10? NO → fall.
;   arm2 pattern (Lo w) FAILS (scrutinee is Hi) → guard NOT evaluated (no perform).
;   arm3 (guard (Hi h2) (> (+ h2 (Ctr.next)) 5)): pattern matches → next reads 1 (state→2) → 7+1=8 > 5? YES → h2*10 = 70.
;   final (Ctr.next) reads 2 → total 70 + 2 = 72.
;   k=2 → (Lo 2): arm1 pattern fails (no perform); arm2 (Lo w) matches → guard (> (+ w (Ctr.next)) 1):
;     next reads 0 → 2+0=2 > 1? YES → w*100 = 200; final next reads 1 → 201.

(case "a performing guard fires once per TRIED arm in order and skips pattern-failed arms"
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (type Sz (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (def (main (: k Int64))
              (handle Ctr 0 ((next (u) s (resume s (+ s 1))))
                (+ (match (mk k)
                     ((guard (Hi h) (> (+ h (Ctr.next)) 10)) (- 0 h))
                     ((guard (Lo w) (> (+ w (Ctr.next)) 1)) (* w 100))
                     ((guard (Hi h2) (> (+ h2 (Ctr.next)) 5)) (* h2 10))
                     (_ -999))
                   (Ctr.next))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 72 Int64))
  (call   main (: 2 Int64)) (output (: 201 Int64)))
