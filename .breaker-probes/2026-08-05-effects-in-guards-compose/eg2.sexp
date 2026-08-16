(case "eg2 BOOLEAN operators short-circuit ACROSS performs: (and (pred (St.get)) (pred2 (St.get)))"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get (u) s (resume s (+ s 1))))
                (+ (if (and (> (St.get) 3) (> (St.get) 10)) 100 10)
                   (St.get))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 17 Int64)))
