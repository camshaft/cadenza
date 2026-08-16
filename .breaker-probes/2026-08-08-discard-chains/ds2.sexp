(case "ds2 the DISCARDED statement is itself an op-result chain — both hops advance the thread even though the value dies"
  (input  (do
            (effect E (op a (-> Int64)) (op b (-> Int64 Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (probe () s (resume s s)))
                (do (E.b (E.a)) (E.probe))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64))
  (call   main (: 0 Int64)) (output (: 5 Int64))
  (call   main (: -9 Int64)) (output (: -4 Int64)))
