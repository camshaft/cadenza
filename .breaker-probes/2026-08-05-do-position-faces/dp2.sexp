(case "dp2 performs in DISCARDED do positions still run (side-effect-only statements advance state)"
  (input  (do
            (effect St (op a (-> Unit Int64)) (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume 0 (+ s 1)))
                 (get (u) s (resume s s)))
                (do
                  (St.a)
                  (St.a)
                  (St.a)
                  (St.get))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 8 Int64)))
