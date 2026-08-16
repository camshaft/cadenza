(case "sk2e dissect: Map state, halt-ONLY (no puts, no resume anywhere)"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St Map.empty
                ((halt (u) s (* 1000 (+ (Map.len s) a))))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 2000 Int64)))
