(case "rp5 a state-REPLACING op AFTER the two-site performs folds (reset-at-end)"
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op reset (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (reset (u) s (resume s 100)))
                (+ (St.sift 20) (+ (St.sift n) (St.reset)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 52 Int64)))
