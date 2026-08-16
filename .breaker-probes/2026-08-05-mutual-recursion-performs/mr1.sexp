(case "mr1 MUTUALLY recursive helpers BOTH perform against the same handler (even/odd tagged drain)"
  (input  (do
            (effect Cnt (op tick (-> Unit Int64)))
            (def (evens (: k Int64))
              (if (= k 0) 0 (+ (* 10 (Cnt.tick)) (odds (- k 1)))))
            (def (odds (: k Int64))
              (if (= k 0) 0 (+ (Cnt.tick) (evens (- k 1)))))
            (def (main (: n Int64))
              (handle Cnt n
                ((tick (u) s (resume s (+ s 1))))
                (evens 4)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 134 Int64)))
