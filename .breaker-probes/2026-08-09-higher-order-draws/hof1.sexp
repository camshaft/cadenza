(case "hof1 a higher-order APPLY-TWICE over a draw — the fn value crosses the call while the thread advances underneath"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (dbl1 (: x Int64)) (+ (* 2 x) 1))
            (def (twice (: f (-> Int64 Int64)) (: x Int64)) (f (f x)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (* 10 (twice dbl1 (E.next))) (- (E.probe) n))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 151 Int64))
  (call   main (: 0 Int64)) (output (: 31 Int64))
  (call   main (: -4 Int64)) (output (: -129 Int64)))
