(case "ta3a a THREE-argument op — the arm folds all three positions with the live state, two calls see it advance"
  (input  (do
            (effect E (op mix3 (-> Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((mix3 (a b c) s (resume (+ (* 100 a) (+ (* 10 b) (+ c s))) (+ s 1))))
                (+ (E.mix3 1 2 3) (E.mix3 4 5 6))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 580 Int64))
  (call   main (: 5 Int64)) (output (: 590 Int64))
  (call   main (: -3 Int64)) (output (: 574 Int64)))
