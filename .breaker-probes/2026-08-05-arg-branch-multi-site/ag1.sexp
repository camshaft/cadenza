(case "ag1 a TWO-resume-site arm branching on the OP ARGUMENT (threshold sift) with a hit-count state"
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (count (u) s (resume s s)))
                (+ (St.sift 20) (+ (St.sift n) (+ (St.sift 30) (* 1000 (St.count)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2050 Int64)))
