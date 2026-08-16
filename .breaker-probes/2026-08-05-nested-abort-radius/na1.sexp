(case "na1 an abort whose VALUE is computed by performing a RESUMING op of the SAME handler (abort-arm self-perform)"
  (input  (do
            (effect St (op get (-> Unit Int64)) (op halt (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get (u) s (resume s s))
                 (halt (u) s (* 100 s)))
                (+ 5 (St.halt))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 700 Int64)))
