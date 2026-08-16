(case "g4 guard TRUE path: perform-result scrutinee passes the guard (no fallback entry)"
  (input  (do
            (effect St (op roll (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((roll (u) s (resume s (+ s 3))))
                (match (St.roll)
                  ((guard v (> v 6)) (* v 100))
                  (v v))))
            (export main)))
  (call   main (: 9 Int64)) (output (: 900 Int64)))
