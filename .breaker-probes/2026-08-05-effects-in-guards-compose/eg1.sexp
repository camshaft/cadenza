(case "eg1 a perform result FEEDS a guard-like predicate helper which gates a SECOND perform"
  (input  (do
            (effect St (op get (-> Unit Int64)) (op act (-> Unit Int64)))
            (def (big (: x Int64)) (> x 3))
            (def (main (: n Int64))
              (handle St n
                ((get (u) s (resume s (+ s 1)))
                 (act (u) s (resume (* 100 s) s)))
                (if (big (St.get)) (St.act) -1)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 600 Int64)))
