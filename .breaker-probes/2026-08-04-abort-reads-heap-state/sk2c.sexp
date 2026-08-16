(case "sk2c dissect: MAP state (Int keys) with the same two-op abort shape"
  (input  (do
            (effect St (op put (-> Int64 Int64)) (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St Map.empty
                ((put (v) s (resume (Map.len s) (Map.insert s v v)))
                 (halt (u) s (* 1000 (Map.len s))))
                (do
                  (def l1 (St.put a))
                  (def l2 (St.put (+ a 1)))
                  (+ (St.halt) (St.put 99)))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 2000 Int64)))
