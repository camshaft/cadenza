(case "bg-sound a scrutinee FAILING the pattern reaches the catch-all WITHOUT running the guard's perform"
  (input  (do
            (effect St (op quota (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((quota (u) s (resume s (+ s 1))))
                (+ (* 100 (match (None unit)
                            ((guard (Some v) (> v (St.quota))) v)
                            (_other 99)))
                   (St.quota))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9905 Int64)))
