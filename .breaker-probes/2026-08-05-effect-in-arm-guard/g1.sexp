(case "g1 a guard reads the perform-result scrutinee; fallback arm is PURE"
  (input  (do
            (effect St (op roll (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((roll (u) s (resume s (+ s 3))))
                (match (St.roll)
                  ((guard v (> v 6)) (* v 100))
                  (v v))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
