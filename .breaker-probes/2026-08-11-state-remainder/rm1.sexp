(case "rm1 remainder BY the state — truncated dividend-sign semantics hold as the state walks through negative and positive divisors"
  (input  (do
            (effect S (op rem (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S n
                ((rem (v) s (resume (% v s) (+ s 3))))
                (let ((a (S.rem -7)))
                  (let ((b (S.rem 7)))
                    (+ a (* 100 b))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 99 Int64))
  (call   main (: -5 Int64)) (output (: 98 Int64))
  (call   main (: 2 Int64)) (output (: 199 Int64)))
