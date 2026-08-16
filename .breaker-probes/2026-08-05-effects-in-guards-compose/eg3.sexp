(case "eg3 short-circuit SKIPS the second perform: (or (pred (St.get)) (pred (St.get))) with first true"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get (u) s (resume s (+ s 1))))
                (+ (if (or (> (St.get) 3) (> (St.get) 0)) 100 10)
                   (St.get))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 106 Int64)))
