(case "sy7 a SYMBOL mode BESIDE an accumulator in the tuple state — symbol equality routes the go arm between idle and run, stop resets the mode"
  (input  (do
            (effect E (op go (-> Int64)) (op stop (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple (Symbol.of "idle") n)
                ((go () st (match st
                             ((tuple m acc)
                              (if (= m (Symbol.of "idle"))
                                  (resume 1 (tuple (Symbol.of "run") acc))
                                  (resume (+ acc 10) (tuple m (+ acc 10)))))))
                 (stop () st (match st
                               ((tuple m acc) (resume acc (tuple (Symbol.of "idle") acc))))))
                (+ (E.go)
                   (+ (* 10 (E.go))
                      (+ (* 100 (E.stop))
                         (* 1000 (E.go)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2651 Int64))
  (call   main (: 0 Int64)) (output (: 2101 Int64))
  (call   main (: -3 Int64)) (output (: 1771 Int64)))
