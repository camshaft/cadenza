(case "ti1 TWO independent effects interleaved in one expression: state isolation under alternating performs"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a (u) s (resume s (+ s 1))))
                (handle B 100
                  ((b (u) t (resume t (+ t 10))))
                  (+ (A.a) (+ (B.b) (+ (A.a) (B.b)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 221 Int64)))
