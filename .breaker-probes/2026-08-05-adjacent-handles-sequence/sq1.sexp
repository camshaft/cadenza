(case "sq1 SEQUENTIAL handles of the same effect in one do: each gets a fresh state, results accumulate"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def r1 (handle St n ((a (u) s (resume s (+ s 1)))) (+ (St.a) (St.a))))
                (def r2 (handle St (* n 10) ((a (u) s (resume s (+ s 1)))) (+ (St.a) (St.a))))
                (+ (* 100 r1) r2)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 761 Int64)))
