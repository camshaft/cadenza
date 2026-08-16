(case "ch4 the SAME op chained into itself — E.b of E.b of E.a, the middle hop's argument is already a hop"
  (input  (do
            (effect E (op a (-> Int64)) (op b (-> Int64 Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (probe () s (resume s s)))
                (+ (* 10 (E.b (E.b (E.a)))) (- (E.probe) n))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 138 Int64))
  (call   main (: 0 Int64)) (output (: 78 Int64))
  (call   main (: -5 Int64)) (output (: -72 Int64)))
