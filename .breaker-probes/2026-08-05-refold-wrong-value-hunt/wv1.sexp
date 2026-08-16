(case "wv1 a MIXED arm: one branch resumes, the other ABORTS (multi-site with an abort site)"
  (input  (do
            (effect St (op check (-> Int64 Int64)))
            (def (main (: n Int64))
              (+ 1000
                (handle St 0
                  ((check (v) s (if (> v 3) s (resume v (+ s 1)))))
                  (+ (St.check n) (+ (St.check 2) (St.check 9))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1002 Int64)))
