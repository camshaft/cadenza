(case "ht2 heap state through TWO chained helpers in an abort arm (depth-2 of the sk11 face)"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (inner (: xs (List Int64))) (List.len xs))
            (def (outer (: xs (List Int64))) (* 1000 (inner xs)))
            (def (main (: a Int64))
              (handle St (list 1 2 3)
                ((halt (u) s (outer s)))
                (St.halt)))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 3000 Int64)))
