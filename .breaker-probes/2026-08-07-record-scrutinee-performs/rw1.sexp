(case "rw1 a record-literal scrutinee with TWO perform fields — field eval order locked to dispatch order"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (match (record (a (St.next)) (b (St.next)))
                  ((record (a x) (b y)) (+ (* 10 x) y)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
