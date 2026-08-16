(case "sg1 the arm consults a CAPTURED Set (from main's scope) to gate its answer — arm reads enclosing heap"
  (input  (do
            (effect St (op check (-> Int64 Int64)))
            (def (main (: n Int64))
              (do
                (def allow (Set.of (list 2 5 9)))
                (handle St 0
                  ((check (v) s (resume (if (Set.contains allow v) 1 0) s)))
                  (+ (* 100 (St.check n)) (+ (* 10 (St.check 3)) (St.check 9))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 101 Int64)))
