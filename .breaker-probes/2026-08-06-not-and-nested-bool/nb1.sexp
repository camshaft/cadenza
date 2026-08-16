(case "nb1 a perform under NOT in a condition (the negated dispatch gate)"
  (input  (do
            (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((check (u) s (resume s (+ s 1)))
                 (count (u) s (resume s s)))
                (if (not (> (St.check) 3))
                  (* 100 (St.count))
                  (St.count))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 200 Int64)))
