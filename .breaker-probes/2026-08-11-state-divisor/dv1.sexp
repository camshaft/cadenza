(case "dv1 the STATE is the divisor — a descending thread crosses zero and the exact dispatch that hits it traps"
  (input  (do
            (effect S (op div (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S n
                ((div (v) s (resume (/ v s) (- s 1))))
                (let ((a (S.div 100)))
                  (let ((b (S.div 100)))
                    (+ a (* 1000 b))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 25020 Int64))
  (call   main (: 2 Int64)) (output (: 100050 Int64))
  (call   main (: 1 Int64)) (trap "divide by zero"))
