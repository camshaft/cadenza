(case "sk2h dissect: Map state, abort arm returns CONSTANT (no state read)"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St Map.empty
                ((halt (u) s (* 1000 a)))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 2000 Int64)))
