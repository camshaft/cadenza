(case "gr2 guarded match on a perform result INSIDE a two-site arm's served branch"
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op roll (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (roll (u) s (resume s (+ s 3))))
                (match (St.roll)
                  ((guard v (> v 6)) (* v 100))
                  (v (+ (St.sift 20) (+ (St.sift 3) v))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 25 Int64)))
