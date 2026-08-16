(case "nt1 a NESTED tuple state (a (b c)) — the arm destructures two levels and rebuilds with three different strides"
  (input  (do
            (effect E (op sum (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (tuple 100 7000))
                ((sum () s (match s
                             ((tuple a inner)
                               (match inner
                                 ((tuple b c)
                                   (resume (+ a (+ b c))
                                           (tuple (+ a 1) (tuple (+ b 10) (+ c 700))))))))))
                (+ (E.sum) (* 10 (E.sum)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 85243 Int64))
  (call   main (: 0 Int64)) (output (: 85210 Int64))
  (call   main (: -6 Int64)) (output (: 85144 Int64)))
