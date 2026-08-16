(case "tg1 a guard predicate PERFORMS and the arm body reads the ADVANCED state (guard-perform ordering)"
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((tick (u) s (resume s (+ s 1))))
                (match 9
                  ((guard v (> (St.tick) 3)) (+ (* 100 v) (St.tick)))
                  (_v 777))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 906 Int64)))
