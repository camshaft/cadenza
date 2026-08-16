(case "th1 a THREE-site arm (nested if, three resume sites) folds"
  (input  (do
            (effect St (op rank (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((rank (v) s
                  (if (> v 20) (resume (* v 10) (+ s 100))
                    (if (> v 10) (resume v (+ s 1)) (resume 0 s)))))
                (+ (St.rank 25) (+ (St.rank 15) (St.rank n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 265 Int64)))
