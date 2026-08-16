(case "gi2 a generic PAIR-SWAPPER over tuples of perform results at two element types"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (swap p) (tuple (. p 1) (. p 0)))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume s (+ s 1))))
                (do
                  (def t1 (swap (tuple (St.a) (St.a))))
                  (def t2 (swap (tuple "x" "y")))
                  (+ (* 100 (. t1 0)) (+ (* 10 (. t1 1)) (String.scalar-len (. t2 0)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 651 Int64)))
