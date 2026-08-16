(case "mf1 a helper performing TWO different effects — both handlers discharge it, both states advance per call"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (def (both) (+ (A.a) (* 10 (B.b))))
            (def (main (: n Int64))
              (handle A n
                ((a () s (resume s (+ s 1))))
                (handle B 100
                  ((b () t (resume t (* t 2))))
                  (+ (both) (both)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3011 Int64))
  (call   main (: 0 Int64)) (output (: 3001 Int64)))
