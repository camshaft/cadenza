; breaker probe Z — the WRONGLY-dead hazard of range-based dead-arm elimination (52a834dac pins
; that dead arms DROP; this pins that live-but-looks-dead arms SURVIVE):
; (% x 8) over SIGNED x has range [-7,7] — truncating % takes the DIVIDEND's sign. An arm probing
; -7 is LIVE (x=-7 → -7%8 = -7). A range analysis that modeled % as [0,7] (unsigned intuition)
; would drop the -7 arm → silent wrong answer at x=-7, exactly the class arm_is_dead must not hit.
; Also the & mask BOUNDARY: (& x 7) arm at probe 7 (range MAX, inclusive) is live at x=7 — an
; exclusive-max off-by-one would drop it.
; Hand-derived: f(-7): -7 % 8 = -7 → arm -7 → 1. f(3): 3%8=3 → wildcard → 0.
;   g(7): 7&7=7 → arm 7 → 1. g(6): 6&7=6 → wildcard → 0.
;   main k: (+ (* 1000 (f (- 0 k))) (+ (* 100 (f 3)) (+ (* 10 (g k)) (g 6)))) at k=7:
;   f(-7)=1, f(3)=0, g(7)=1, g(6)=0 → 1000 + 0 + 10 + 0 = 1010.

(case "range-based dead-arm elimination keeps the signed-mod negative arm and the mask max arm"
  (input  (do
            (def (f (: x Int64)) (match (% x 8) (-7 1) (_ 0)))
            (def (g (: x Int64)) (match (& x 7) (7 1) (_ 0)))
            (def (main (: k Int64))
              (+ (* 1000 (f (- 0 k)))
                 (+ (* 100 (f 3))
                    (+ (* 10 (g k)) (g 6)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 1010 Int64)))
