(case "abf1 one handler mixes a RESUMPTIVE op and an ABORTIVE op — two marks advance the state, the abort carries their mix out"
  (input  (do
            (effect Bail (op mark (-> Int64)) (op out (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Bail n
                ((mark () t (resume t (+ t 5)))
                 (out (v) t (+ 1000 v)))
                (let ((m1 (Bail.mark)))
                  (let ((m2 (Bail.mark)))
                    (+ (Bail.out (+ (* 10 m1) m2)) 777)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1038 Int64))
  (call   main (: 0 Int64)) (output (: 1005 Int64))
  (call   main (: -4 Int64)) (output (: 961 Int64)))
