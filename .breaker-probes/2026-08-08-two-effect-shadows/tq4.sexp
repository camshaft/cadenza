(case "tq4 BOTH effects shadowed in one inner region — two fresh threads run their course, both outer threads untouched"
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
                        (+ (handle P 40
                             ((next () s (resume s (+ s 4))))
                             (handle Q 7000
                               ((next () t (resume t (+ t 700))))
                               (+ (P.next) (+ (Q.next) (P.next)))))
                           (+ (P.next) (Q.next))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7305 Int64))
  (call   main (: 0 Int64)) (output (: 7295 Int64))
  (call   main (: -9 Int64)) (output (: 7277 Int64)))
