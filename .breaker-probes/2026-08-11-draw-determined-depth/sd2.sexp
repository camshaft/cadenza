(case "sd2 the recursion DEPTH itself is a draw — the walk runs mod-4-of-state iterations, including the zero-depth face"
  (input  (do
            (effect S (op depth (-> Int64)) (op tick (-> Int64)))
            (def (walk (: k Int64) (: acc Int64))
              (if (< k 1) acc (walk (- k 1) (+ (* 10 acc) (S.tick)))))
            (def (main (: n Int64))
              (handle S n
                ((depth () s (resume (% s 4) (+ s 1)))
                 (tick () s (resume s (+ s 1))))
                (let ((d (S.depth)))
                  (+ (* 100000 d) (walk d 0)))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 200078 Int64))
  (call   main (: 4 Int64)) (output (: 0 Int64))
  (call   main (: 5 Int64)) (output (: 100006 Int64)))
