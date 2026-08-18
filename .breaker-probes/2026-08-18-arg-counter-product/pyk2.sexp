(case "pyk2 the TOLL MULTIPLIES THE OP ARGUMENT BY THE DISPATCH COUNTER — every capture source meets in one product with the argument from the perform site and the counter from the tuple state, each unwinding frame recalls ITS OWN pair so the first frame pays three-times-one and the second five-times-two, and any cross-frame mixing of argument and counter misprices the thousands"
  (input  (do
            (effect E (op tick (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (tuple (% n 3) (: 0 Int64))
                ((tick (v) st
                  (match st
                    ((tuple base k)
                      (+ (resume (+ base v) (tuple (+ base v) (+ k 1)))
                         (* 1000 (* v (+ k 1))))))))
                (let ((a (E.tick 3)))
                  (let ((b (E.tick 5)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 13094 Int64))
  (call   main (: 0 Int64)) (output (: 13083 Int64)))
