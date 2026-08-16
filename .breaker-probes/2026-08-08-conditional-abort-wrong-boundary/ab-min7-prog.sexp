(do
            (effect A (op out (-> Int64 Int64)))
            (effect R (op get (-> Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((out (v) t (+ 9000 v)))
                (+ (* 100 (handle R 5
                            ((get () t (resume t t)))
                            (if (> n 0) (A.out n) (R.get))))
                   7)))
            (export main))
