(case "bo1 performs in BOTH and-operands — the second fires when the first passes"
  (input  (do
            (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((check (u) s (resume s (+ s 1)))
                 (count (u) s (resume s s)))
                (if (and (> (St.check) 3) (> (St.check) 4))
                  (* 100 (St.count))
                  (St.count))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 700 Int64)))
