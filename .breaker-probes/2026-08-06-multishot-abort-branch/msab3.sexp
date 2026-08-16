(case "msab3 the conditional-count arm's SINGLE-shot branch (same program, other input)"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb n
                ((flip (u) s (if (> s 3) (+ (resume 1 s) (resume 2 s)) (resume 9 s))))
                (+ (Amb.flip) 5)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 14 Int64)))
