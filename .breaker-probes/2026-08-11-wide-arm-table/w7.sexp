(case "w7 a SEVEN-op handler — the widest arm table in the corpus, each op a distinct answer shape and stride, order scrambled"
  (input  (do
            (effect W
              (op o1 (-> Int64)) (op o2 (-> Int64)) (op o3 (-> Int64))
              (op o4 (-> Int64)) (op o5 (-> Int64)) (op o6 (-> Int64)) (op o7 (-> Int64)))
            (def (main (: n Int64))
              (handle W n
                ((o1 () s (resume (+ s 1) (+ s 1)))
                 (o2 () s (resume (* s 2) (+ s 2)))
                 (o3 () s (resume (- s 3) (+ s 3)))
                 (o4 () s (resume (* s s) (+ s 4)))
                 (o5 () s (resume (% s 5) (+ s 5)))
                 (o6 () s (resume (/ s 2) (+ s 6)))
                 (o7 () s (resume (- 0 s) (+ s 7))))
                (+ (W.o1)
                   (+ (* 10 (W.o3))
                      (+ (* 100 (W.o5))
                         (+ (* 1000 (W.o7))
                            (+ (* 10000 (W.o2))
                               (+ (* 100000 (W.o4)) (* 10000000 (W.o6))))))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 160349103 Int64))
  (call   main (: 0 Int64)) (output (: 142711381 Int64)))
