(case "ag3 one resumptive dispatch then an ABORT on the same handler — the abort arm reads the advanced state"
  (input  (do
            (effect St (op put (-> Int64 Int64)) (op halt (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((put (v) s (resume s (+ s v)))
                 (halt (u) s (* 100 s)))
                (match (St.put n) (_ (+ 7777 (St.halt))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 300 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64))
  (call   main (: -4 Int64)) (output (: -400 Int64)))
