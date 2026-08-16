(case "sk5 heap seed READ by abort arm while ANOTHER op resumed EARLIER (mixed lifecycle)"
  (input  (do
            (effect St (op put (-> Int64 Int64)) (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (list)
                ((put (v) s (resume 0 (List.push s v)))
                 (halt (u) s (* 1000 (List.len s))))
                (do
                  (def x (St.put a))
                  (St.halt))))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 1000 Int64)))
