(case "wc1 the inner arm's resume value draws from TWO different outer effects — both threads advance per inner dispatch"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (effect I (op ask (-> Int64)))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (handle I 0
                    ((ask () u (resume (+ (P.next) (Q.next)) u)))
                    (+ (I.ask) (* 1000 (I.ask)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 114103 Int64))
  (call   main (: 0 Int64)) (output (: 111100 Int64))
  (call   main (: -7 Int64)) (output (: 104093 Int64)))
