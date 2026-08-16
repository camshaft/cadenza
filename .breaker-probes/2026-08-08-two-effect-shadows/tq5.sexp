(case "tq5 a DEF installs the Q-shadow and draws P from inside it — the P dispatch crosses the def AND the shadow to the caller's frame"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (def (qshadow)
              (handle Q 9000
                ((next () t (resume t (+ t 9))))
                (+ (Q.next) (P.next))))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (+ (P.next)
                     (+ (qshadow)
                        (+ (Q.next) (P.next)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9118 Int64))
  (call   main (: 0 Int64)) (output (: 9103 Int64))
  (call   main (: -6 Int64)) (output (: 9085 Int64)))
