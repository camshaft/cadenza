(case "ms3b control: multi-shot x FLAT two performs (no recursion): (* (Amb.flip) (Amb.flip))"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((flip (u) s (+ (resume 2 s) (resume 3 s))))
                (* (Amb.flip) (Amb.flip))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 25 Int64)))
