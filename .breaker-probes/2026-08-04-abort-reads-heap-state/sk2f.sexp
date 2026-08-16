(case "sk2f dissect: Map state, resume-only ops (no abort arm at all)"
  (input  (do
            (effect St (op put (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle St Map.empty
                ((put (v) s (resume (Map.len s) (Map.insert s v v))))
                (do
                  (def l1 (St.put a))
                  (def l2 (St.put (+ a 1)))
                  (+ (* 10 l1) l2))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 1 Int64)))
