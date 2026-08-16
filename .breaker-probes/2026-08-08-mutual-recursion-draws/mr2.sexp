(case "mr2 the mutual pair draws from DIFFERENT effects — ev advances P, od advances Q, both threads interleave down the descent"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (def (ev (: k Int64))
              (if (<= k 0) 0 (+ (* 10 (P.next)) (od (- k 1)))))
            (def (od (: k Int64))
              (if (<= k 0) 0 (+ (Q.next) (ev (- k 1)))))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (ev 4))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 260 Int64))
  (call   main (: 0 Int64)) (output (: 220 Int64))
  (call   main (: -5 Int64)) (output (: 120 Int64)))
