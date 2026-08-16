(case "ee2 the tuple-wrapped closure factory called TWICE (two factory dispatches, distinct captures)"
  (input  (do
            (effect Mk (op make (-> Int64 (Tuple (-> Int64 Int64) Int64))))
            (def (main (: k Int64))
              (handle Mk 10
                ((make (base) s (resume (tuple (fn ((: x Int64)) (+ x (+ base s))) base) (+ s 1))))
                (match (Mk.make k)
                  ((tuple f _b)
                    (match (Mk.make (* k 10))
                      ((tuple g _c) (+ (f 0) (g 0))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 76 Int64)))
