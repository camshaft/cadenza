(case "or2 a false first operand falls through — the OR's second perform fires (same program)"
  (input  (do
            (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((check (u) s (resume s (+ s 1)))
                 (count (u) s (resume s s)))
                (if (or (> (St.check) 3) (> (St.check) 0))
                  (St.count)
                  (* 100 (St.count)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 3 Int64)))
