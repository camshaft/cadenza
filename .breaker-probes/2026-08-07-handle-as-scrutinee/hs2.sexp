(case "hs2 an OUTER-seeded inner handle builds a tuple scrutinee — the destructured arm re-performs against the outer"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (match (handle St (St.next)
                         ((next () t (resume t (* t 2))))
                         (tuple (St.next) (St.next)))
                  ((tuple a b) (+ (* 100 a) (+ (* 10 b) (St.next)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 606 Int64))
  (call   main (: 2 Int64)) (output (: 243 Int64)))
