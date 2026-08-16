(case "sk2d dissect: Map state + abort WITHOUT the dead-suffix perform (halt in tail position)"
  (input  (do
            (effect St (op put (-> Int64 Int64)) (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St Map.empty
                ((put (v) s (resume (Map.len s) (Map.insert s v v)))
                 (halt (u) s (* 1000 (Map.len s))))
                (do
                  (def l1 (St.put a))
                  (def l2 (St.put (+ a 1)))
                  (St.halt))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 2000 Int64)))
