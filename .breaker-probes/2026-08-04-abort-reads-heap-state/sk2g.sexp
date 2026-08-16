(case "sk2g dissect: LIST state, abort arm READS the state (List.len s)"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (list)
                ((halt (u) s (* 1000 (+ (List.len s) a))))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 2000 Int64)))
