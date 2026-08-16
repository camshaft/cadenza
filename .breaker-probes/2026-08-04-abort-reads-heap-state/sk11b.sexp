(case "sk11b sk11 boundary: SCALAR state read through a helper in an ABORT arm"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (score (: s Int64)) (* 1000 s))
            (def (main (: a Int64))
              (handle St 2
                ((halt (u) s (score s)))
                (St.halt)))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 2000 Int64)))
