(case "th4 a THREE-site and a TWO-site arm interleaved fold (multi-multi mixing serves)"
  (input  (do
            (effect St (op rank (-> Int64 Int64)) (op sift (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((rank (v) s
                  (if (> v 20) (resume (* v 10) (+ s 100))
                    (if (> v 10) (resume v (+ s 1)) (resume 0 s))))
                 (sift (v) s (if (> v 5) (resume v (+ s 10)) (resume -1 s))))
                (+ (St.rank 25) (+ (St.sift 7) (St.rank n)))))
            (export main)))
  (call   main (: 15 Int64)) (output (: 272 Int64)))
