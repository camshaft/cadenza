(case "nx1 NEGATIVE and HUGE indices cross the dispatch — List.at in the arm answers None-fallback for both out-of-range directions"
  (input  (do
            (effect S (op at (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (list 10 20 30)
                ((at (i) s (resume (match (List.at s i) ((Some v) v) ((None _u) -7)) s)))
                (+ (* 10000 (S.at n))
                   (+ (* 100 (S.at -1)) (S.at 99)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 199293 Int64))
  (call   main (: -5 Int64)) (output (: -70707 Int64)))
