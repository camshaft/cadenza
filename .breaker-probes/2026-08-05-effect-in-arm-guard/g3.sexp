(case "g3 the GUARD CONDITION itself performs (pure scrutinee)"
  (input  (do
            (effect St (op roll (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((roll (u) s (resume s (+ s 3))))
                (match n
                  ((guard v (> (St.roll) 4)) (* v 100))
                  (v v))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 500 Int64)))
