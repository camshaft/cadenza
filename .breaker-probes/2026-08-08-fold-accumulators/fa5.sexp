(case "fa5 a TWO-effect fold — each level's step multiplies a draw from each thread, both threads advancing independently"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (def (fold (: k Int64) (: acc Int64))
              (if (<= k 0)
                  acc
                  (fold (- k 1) (+ acc (* (P.next) (Q.next))))))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (fold 3 0))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 1010 Int64))
  (call   main (: 0 Int64)) (output (: 350 Int64))
  (call   main (: -1 Int64)) (output (: 20 Int64)))
