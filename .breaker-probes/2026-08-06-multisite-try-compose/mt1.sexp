(case "mt1 a multi-site arm's resume value feeds a CONST-succeeding try in the body"
  (input  (do
            (effect St (op sift (-> Int64 Int64)))
            (def (classify (: x Int64))
              (do
                (def v (try (Some x)))
                (Some (+ v 1))))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s))))
                (match (classify (St.sift 20))
                  ((Option.Some r) (+ r (St.sift n)))
                  ((Option.None _u) -1))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 51 Int64)))
