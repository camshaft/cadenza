(case "rw4 tuple-literal scrutinee control — same shape with a tuple"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (match (tuple (St.next) (St.next))
                  ((tuple x y) (+ (* 10 x) y)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
