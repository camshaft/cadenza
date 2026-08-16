(case "pa1 the abort arm reads heap state AND the abort value flows into a MATCH in the program"
  (input  (do
            (effect St (op halt (-> Unit (Option Int64))))
            (def (main (: a Int64))
              (match (handle St (list a (+ a 1))
                       ((halt (u) s (if (> (List.len s) 1) (Option.Some (List.len s)) (Option.None))))
                       (St.halt))
                ((Option.Some v) (* 100 v))
                ((Option.None) -1)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 200 Int64)))
