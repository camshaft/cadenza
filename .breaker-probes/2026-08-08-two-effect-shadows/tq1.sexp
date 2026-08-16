(case "tq1 a Q-shadow inside a TWO-effect region — P draws thread THROUGH the shadow while Q is locally rebound"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (+ (P.next)
                     (+ (Q.next)
                        (+ (handle Q 7000
                             ((next () t (resume t (+ t 100))))
                             (+ (Q.next) (P.next)))
                           (Q.next)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7221 Int64))
  (call   main (: 0 Int64)) (output (: 7211 Int64))
  (call   main (: -8 Int64)) (output (: 7195 Int64)))
