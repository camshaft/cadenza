(case "sk2 the sum-keyed Map state SURVIVES an abort: pre-abort inserts visible, post-abort perform discarded"
  (input  (do
            (type T (TI Int64) (TP T T))
            (effect St (op put (-> Int64 Int64)) (op halt (-> Unit Int64)))
            (def (mk (: i Int64)) (T.TP (T.TI i) (T.TI (* 2 i))))
            (def (main (: a Int64))
              (handle St Map.empty
                ((put (v) s (resume (Map.len s) (Map.insert s (mk v) v)))
                 (halt (u) s (* 1000 (Map.len s))))
                (do
                  (def l1 (St.put a))
                  (def l2 (St.put (+ a 1)))
                  (+ (St.halt) (St.put 99)))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 2000 Int64)))
