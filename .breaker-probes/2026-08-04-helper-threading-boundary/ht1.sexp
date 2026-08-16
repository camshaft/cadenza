(case "ht1 heap OP-ARG (not state) through a helper inside an abort arm"
  (input  (do
            (effect St (op halt (-> (List Int64) Int64)))
            (def (score (: xs (List Int64))) (* 1000 (List.len xs)))
            (def (main (: a Int64))
              (handle St 0
                ((halt (xs) s (score xs)))
                (St.halt (list a (+ a 1)))))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 2000 Int64)))
