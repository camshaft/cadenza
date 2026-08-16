(case "fa2 TWO accumulators through one performing recursion — running sum and prefix-sum-of-prefix-sums stay in step with the draws"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (fold2 (: k Int64) (: a Int64) (: b Int64))
              (if (<= k 0)
                  (+ (* 100 a) b)
                  (let ((d (E.next)))
                    (fold2 (- k 1) (+ a d) (+ b (+ a d))))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (fold2 3 0 0)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 610 Int64))
  (call   main (: 0 Int64)) (output (: 304 Int64))
  (call   main (: -1 Int64)) (output (: -2 Int64)))
