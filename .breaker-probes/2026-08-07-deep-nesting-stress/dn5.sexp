(case "dn5 a tuple built INSIDE the inner region from BOTH effects' draws, destructured OUTSIDE — data crosses the region boundary"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a () s (resume s (+ s 1))))
                (match (handle B 50
                         ((b () t (resume t (* t 2))))
                         (tuple (B.b) (+ (A.a) (B.b))))
                  ((tuple x y) (+ (* 100 x) (+ (* 10 y) (A.a)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6056 Int64))
  (call   main (: 0 Int64)) (output (: 6001 Int64)))
