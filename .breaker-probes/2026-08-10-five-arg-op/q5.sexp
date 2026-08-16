(case "q5 a FIVE-argument op — the arm folds all five positions with distinct weights, two calls permute the arguments"
  (input  (do
            (effect E (op quint (-> Int64 Int64 Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((quint (a b c d e) s
                  (resume (+ a (+ (* 2 b) (+ (* 3 c) (+ (* 4 d) (+ (* 5 e) s)))))
                          (+ s 1))))
                (+ (* 100 (E.quint 1 2 3 4 5))
                   (E.quint 5 4 3 2 1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 5536 Int64))
  (call   main (: 5 Int64)) (output (: 6041 Int64))
  (call   main (: -3 Int64)) (output (: 5233 Int64)))
