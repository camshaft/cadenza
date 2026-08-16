(case "h1 the handler-state SEED list stays readable in the body after performs advance the state"
  (input  (do
            (effect Acc (op push (-> Int64 Int64)))
            (def (main (: k Int64))
              (let ((seed (list k)))
                (handle Acc seed
                  ((push (v) s (resume (List.len s) (List.push s v))))
                  (let ((a (Acc.push 10)))
                    (let ((b (Acc.push 20)))
                      (+ (+ a b)
                         (* 100 (match (List.at seed 0) ((Some v) v) ((None _u) -1)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 503 Int64)))
