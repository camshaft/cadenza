(case "tg2 a FAILING guard's perform still advances state for the fall-through arm"
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((tick (u) s (resume s (+ s 1))))
                (match 9
                  ((guard v (> (St.tick) 100)) 111)
                  (_v (St.tick)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))
