(case "sk3 RUNTIME-BUILT heap seed (not Map.empty) with a state-reading abort arm"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (Map.insert Map.empty 1 a)
                ((halt (u) s (* 1000 (Map.len s))))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 1000 Int64)))
