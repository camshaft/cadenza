(case "mm1 MUTUALLY-recursive module exports both performing, homed at the importer"
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (module m
              (def (ping (: n Int64) (: acc Int64))
                (if (= n 0) acc (pong (- n 1) (+ (* 10 acc) (Ctr.next unit)))))
              (def (pong (: n Int64) (: acc Int64))
                (if (= n 0) acc (ping (- n 1) (+ (* 10 acc) (* 2 (Ctr.next unit))))))
              (export ping) (export pong))
            (def (main (: k Int64))
              (handle Ctr 1
                ((next (u) s (resume s (+ s 1))))
                ((. m ping) k 0)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 143 Int64)))
