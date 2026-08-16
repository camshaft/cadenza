(case "gv2 a record nested in a tuple scrutinee — bound by position first, record-matched after"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (match (tuple (record (a (St.next))) (St.next))
                  ((tuple r y)
                    (match r ((record (a x)) (+ (* 10 x) y)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
