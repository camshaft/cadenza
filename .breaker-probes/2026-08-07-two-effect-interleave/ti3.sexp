(case "ti3 cross-effect resume-value feed PLUS direct body interleave — B's arm and the body both advance A"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a () s (resume s (* s 2))))
                (handle B 1000
                  ((b () t (resume (+ t (A.a)) (- t 1))))
                  (+ (B.b) (+ (A.a) (B.b))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2034 Int64))
  (call   main (: 1 Int64)) (output (: 2006 Int64)))
