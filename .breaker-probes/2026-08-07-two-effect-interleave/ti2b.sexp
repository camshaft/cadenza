(case "ti2b the inner arm's resume-VALUE performs the outer effect on EVERY dispatch — three dispatches, both states advancing"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a () s (resume s (+ s 1))))
                (handle B 0
                  ((b () t (resume (+ t (A.a)) (+ t 1))))
                  (+ (B.b) (+ (B.b) (B.b))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 21 Int64))
  (call   main (: 2 Int64)) (output (: 12 Int64)))
