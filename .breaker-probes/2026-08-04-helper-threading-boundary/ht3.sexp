(case "ht3 heap state as the SECOND arg of a two-param helper in an abort arm"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (score (: k Int64) (: xs (List Int64))) (* k (List.len xs)))
            (def (main (: a Int64))
              (handle St (list 1 2)
                ((halt (u) s (score 500 s)))
                (St.halt)))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 1000 Int64)))
