(case "es1 a satisfied @ensures on a performing def (single call — the served face)"
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ (ensures (>= ret 100)) (def (f (: x Int64)) (+ x (St.bump))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (f n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))
