(case "sk2b control: same two-op abort shape with INT state"
  (input  (do
            (effect St (op put (-> Int64 Int64)) (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St 0
                ((put (v) s (resume s (+ s 1)))
                 (halt (u) s (* 1000 s)))
                (do
                  (def l1 (St.put a))
                  (def l2 (St.put (+ a 1)))
                  (+ (St.halt) (St.put 99)))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 2000 Int64)))
