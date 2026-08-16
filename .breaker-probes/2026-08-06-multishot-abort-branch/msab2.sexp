(case "msab2 the arm CONDITIONS its shot count — two resumes on one branch, one on the other"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb n
                ((flip (u) s (if (> s 3) (+ (resume 1 s) (resume 2 s)) (resume 9 s))))
                (+ (Amb.flip) 5)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 13 Int64)))
