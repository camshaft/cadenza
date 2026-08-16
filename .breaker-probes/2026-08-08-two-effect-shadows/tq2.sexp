(case "tq2 the Q-shadow's SEED mixes P and Q draws — both outer threads advance before the shadow opens, and resume after"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (+ (handle Q (+ (* 100 (P.next)) (Q.next))
                       ((next () t (resume t t)))
                       (Q.next))
                     (+ (Q.next) (P.next))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 413 Int64))
  (call   main (: 0 Int64)) (output (: 211 Int64))
  (call   main (: -5 Int64)) (output (: -294 Int64)))
