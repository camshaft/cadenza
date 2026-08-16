(case "na2 the abort ARM ITSELF performs the same handler's resuming op ((halt (u) s (* 100 (St.get))))"
  (input  (do
            (effect St (op get (-> Unit Int64)) (op halt (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get (u) s (resume s s))
                 (halt (u) s (* 100 (St.get))))
                (+ 5 (St.halt))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 700 Int64)))
