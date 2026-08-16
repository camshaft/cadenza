(case "rp1 two-site arm + a state-REPLACING second op over a SCALAR state"
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op reset (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (reset (u) s (resume s 100)))
                (+ (St.sift 20) (+ (St.reset) (St.sift 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 51 Int64)))
