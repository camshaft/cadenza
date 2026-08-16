(case "or1 the OR short-circuits on a true first operand — the second perform must NOT fire"
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
  (call   main (: 5 Int64)) (output (: 6 Int64)))
