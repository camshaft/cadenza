(case "rv3 a RESULT state matched per variant with ONE resume per arm (Ok accumulates, Err echoes)"
  (input  (do
            (effect St (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (Result.Ok 0)
                ((add (v) s
                  (match s
                    ((Result.Ok acc) (resume (+ acc v) (Result.Ok (+ acc v))))
                    ((Result.Err e) (resume e (Result.Err e))))))
                (+ (* 100 (St.add n)) (+ (* 10 (St.add 4)) (St.add 2)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 379 Int64)))
