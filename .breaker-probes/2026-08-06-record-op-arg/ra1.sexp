(case "ra1 a RECORD as op ARGUMENT — the arm projects the fields it is handed"
  (input  (do
            (effect St (op score (-> (Record (hits Int64) (misses Int64)) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((score (r) s (resume (- (* (. r hits) 10) (. r misses)) s)))
                (St.score (record (hits n) (misses 3)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 47 Int64)))
