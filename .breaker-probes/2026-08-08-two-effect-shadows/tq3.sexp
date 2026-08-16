(case "tq3 a Q-shadow wraps only the MIDDLE argument of a pure call — its neighbors dispatch to the outer P and Q frames"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (def (sum3 (: a Int64) (: b Int64) (: c Int64)) (+ a (+ b c)))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (sum3 (P.next)
                        (handle Q 9000
                          ((next () t (resume t (+ t 9))))
                          (Q.next))
                        (Q.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9105 Int64))
  (call   main (: 0 Int64)) (output (: 9100 Int64))
  (call   main (: -3 Int64)) (output (: 9097 Int64)))
