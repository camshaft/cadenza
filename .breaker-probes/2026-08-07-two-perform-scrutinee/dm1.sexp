(case "dm1 a tuple scrutinee built from TWO performs — the guard relates both dispatch results"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (match (tuple (St.next) (St.next))
                  ((guard (tuple a b) (= (+ a 1) b)) (+ (* 100 a) b))
                  ((tuple a b) (- 0 (+ a b))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 506 Int64)))
