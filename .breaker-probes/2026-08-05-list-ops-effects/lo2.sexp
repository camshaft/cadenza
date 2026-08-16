(case "lo2 an OUT-OF-BOUNDS perform-computed index into List.at answers None (not trap) under a handler"
  (input  (do
            (effect St (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((pick (u) s (resume s s)))
                (match (List.at (list 10 20) (St.pick))
                  ((Option.Some v) v)
                  ((Option.None) -1))))
            (export main)))
  (call   main (: 9 Int64)) (output (: -1 Int64)))
