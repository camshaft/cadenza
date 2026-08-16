(case "pm4 the LET-BOUND face: a let binding outside the handle is orphaned the same way"
  (input  (do
            (effect St (op price (-> Int64 Int64)))
            (def (main (: n Int64))
              (let ((m (* n 2)))
                (handle St 0
                  ((price (k) s (if (> k 1) (resume 111 (+ s 1)) (resume 100 s))))
                  (+ m (+ (St.price 1) (+ (St.price 7) (St.price 2)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 332 Int64)))
