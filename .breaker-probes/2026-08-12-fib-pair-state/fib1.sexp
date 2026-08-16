(case "fib1 the FIBONACCI recurrence as a state transition — (a,b) becomes (b,a+b) per dispatch, five draws walk the sequence"
  (input  (do
            (effect S (op next (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple 0 1)
                ((next () st
                  (match st ((tuple a b) (resume a (tuple b (+ a b)))))))
                (let ((f1 (S.next)))
                  (let ((_f2 (S.next)))
                    (let ((_f3 (S.next)))
                      (let ((_f4 (S.next)))
                        (let ((f5 (S.next)))
                          (+ (* 1000 f5) (+ (* 10 f1) n)))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 3000 Int64))
  (call   main (: 7 Int64)) (output (: 3007 Int64)))
