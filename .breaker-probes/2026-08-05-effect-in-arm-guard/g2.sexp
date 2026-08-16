(case "g2 plain match on a perform-result scrutinee with a performing fallback (no guard)"
  (input  (do
            (effect St (op roll (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((roll (u) s (resume s (+ s 3))))
                (match (St.roll)
                  (0 999)
                  (v (+ (* 10 (St.roll)) v)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 85 Int64)))
