(case "rv1 a RESULT handler state flips Ok to Err at a threshold and STAYS Err (latching failure)"
  (input  (do
            (effect St (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (Result.Ok 0)
                ((add (v) s
                  (match s
                    ((Result.Ok acc)
                      (if (> (+ acc v) 10)
                        (resume -1 (Result.Err (+ acc v)))
                        (resume (+ acc v) (Result.Ok (+ acc v)))))
                    ((Result.Err e) (resume e (Result.Err e))))))
                (+ (* 1000 (St.add n)) (+ (* 100 (St.add 4)) (+ (* 10 (St.add 9)) (St.add 1))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3706 Int64)))
