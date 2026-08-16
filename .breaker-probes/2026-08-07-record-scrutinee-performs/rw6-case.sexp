(case "rw6 a USER-SUM-literal scrutinee with a performing payload — the ctor pattern binds once"
  (input  (do
            (type Box (Box Int64))
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (* 100 (match (Box.Box (St.next))
                            (((. Box Box) v) v)))
                   (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 506 Int64)))
