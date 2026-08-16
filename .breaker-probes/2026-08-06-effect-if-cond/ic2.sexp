(case "ic2 the false branch: the condition's perform fires, the branch's does NOT (same program)"
  (input  (do
            (effect St (op check (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((check (u) s (resume s (+ s 1))))
                (if (> (St.check) 3) (St.check) 0)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 0 Int64)))
