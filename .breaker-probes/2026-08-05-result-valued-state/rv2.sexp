(case "rv2 the Err PAYLOAD carries a heap List of the rejected inputs (sum-of-heap failure log)"
  (input  (do
            (effect St (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (Result.Ok 0)
                ((add (v) s
                  (match s
                    ((Result.Ok acc)
                      (if (> (+ acc v) 10)
                        (resume 0 (Result.Err (List.push (list) v)))
                        (resume (+ acc v) (Result.Ok (+ acc v)))))
                    ((Result.Err xs) (resume (List.len xs) (Result.Err (List.push xs v)))))))
                (+ (* 100 (St.add n)) (+ (* 10 (St.add 9)) (+ (St.add 2) (St.add 5))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 303 Int64)))
