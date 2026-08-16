(case "oa2 a RESULT as op ARGUMENT — the arm branches on Ok/Err payloads it was handed"
  (input  (do
            (effect St (op judge (-> (Result Int64 Int64) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((judge (r) s (resume (match r ((Result.Ok v) (* v 10)) ((Result.Err e) (- 0 e))) s)))
                (+ (* 100 (St.judge (Result.Ok n)))
                   (St.judge (Result.Err 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4993 Int64)))
