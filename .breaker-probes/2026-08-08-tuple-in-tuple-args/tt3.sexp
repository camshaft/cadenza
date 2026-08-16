(case "tt3 a tuple ROTATED through two chained dispatches — each rotation folds the state into the moved slot"
  (input  (do
            (effect E (op rot (-> (Tuple Int64 Int64 Int64) (Tuple Int64 Int64 Int64))))
            (def (main (: n Int64))
              (handle E n
                ((rot (p) s (match p
                              ((tuple a b c) (resume (tuple b c (+ a s)) (+ s 1))))))
                (match (E.rot (E.rot (tuple n 2 3)))
                  ((tuple x y z) (+ (* 100 x) (+ (* 10 y) z))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 408 Int64))
  (call   main (: 0 Int64)) (output (: 303 Int64)))
