(case "ic1 a perform in the body's IF CONDITION gates a second perform in the branch"
  (input  (do
            (effect St (op check (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((check (u) s (resume s (+ s 1))))
                (if (> (St.check) 3) (St.check) 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))
