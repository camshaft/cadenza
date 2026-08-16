(case "gp5 the guard DESTRUCTURES the tuple payload AND reads the state — (guard (tuple a b) (> (+ a b) s)) admits by the pair's sum against the live threshold"
  (input  (do
            (effect E (op rate (-> (Tuple Int64 Int64) Int64)))
            (def (main (: n Int64))
              (handle E n
                ((rate (p) s
                  (match p
                    ((guard (tuple a b) (> (+ a b) s)) (resume (+ (* 10 a) b) (+ s (+ a b))))
                    ((tuple _a _b) (resume 0 s)))))
                (+ (* 100 (E.rate (tuple 3 4))) (E.rate (tuple 1 2)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 3400 Int64))
  (call   main (: 8 Int64)) (output (: 0 Int64))
  (call   main (: -5 Int64)) (output (: 3412 Int64)))
