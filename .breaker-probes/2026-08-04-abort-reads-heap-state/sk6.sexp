(case "sk6 STRING (rope) state read by an abort arm — does the seed-let-lift class reach String?"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St "seed"
                ((halt (u) s (* 100 (+ (String.scalar-len s) a))))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 600 Int64)))
