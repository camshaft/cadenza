(case "ht4 heap state INLINE-read then result passed to helper: (score (List.len s)) in abort arm"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (score (: n Int64)) (* 1000 n))
            (def (main (: a Int64))
              (handle St (list 1 2)
                ((halt (u) s (score (List.len s))))
                (St.halt)))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 2000 Int64)))
