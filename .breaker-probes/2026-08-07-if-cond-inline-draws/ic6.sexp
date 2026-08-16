(case "ic6 a draw-conditioned branch selects BETWEEN two effects — the untaken effect's state is untouched by that row"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a () s (resume s (+ s 1))))
                (handle B 100
                  ((b () t (resume t (* t 2))))
                  (+ (if (> (A.a) 3) (A.a) (B.b))
                     (* 10 (if (> (A.a) 3) (A.a) (B.b)))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 75 Int64))
  (call   main (: 0 Int64)) (output (: 2100 Int64))
  (call   main (: 2 Int64)) (output (: 2100 Int64)))
