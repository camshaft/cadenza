(case "mc3 multi-shot x two-site arm: the re-reduced continuation ITSELF contains a served branch-arm perform"
  (input  (do
            (effect Go (op fork (-> Unit Int64)))
            (effect St (op sift (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Go 0
                ((fork (u) s (+ (resume 10 s) (resume 20 s))))
                (handle St 0
                  ((sift (v) s (if (> v 15) (resume v (+ s 1)) (resume 0 s))))
                  (+ (St.sift (Go.fork)) n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30 Int64)))
