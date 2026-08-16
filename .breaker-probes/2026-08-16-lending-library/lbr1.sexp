(case "lbr1 a LENDING LIBRARY with a hold queue — borrow hands out a copy while any remain else queues a hold, a RETURN pays a seed-scaled late fine and the copy is INTERCEPTED by the oldest hold (availability untouched) when the queue is non-empty, and the audit packs fines availability and holds; the seed scales only the fine rows while the circulation rows agree"
  (input  (do
            (effect B
              (op borrow (-> Int64))
              (op ret (-> Int64 Int64))
              (op audit (-> Int64)))
            (def (main (: n Int64))
              (handle B (tuple (: 2 Int64) (: 0 Int64) (: 0 Int64))
                ((borrow () st
                  (match st
                    ((tuple avail holds fines)
                      (if (> avail 0)
                          (resume (+ (* (- avail 1) 10) 1) (tuple (- avail 1) holds fines))
                          (resume (+ (* (+ holds 1) 10) 2) (tuple avail (+ holds 1) fines))))))
                 (ret (late) st
                  (match st
                    ((tuple avail holds fines)
                      (if (> holds 0)
                          (resume (+ (* (* late (+ (% n 3) 1)) 100) (+ 90 (- holds 1)))
                                  (tuple avail (- holds 1) (+ fines (* late (+ (% n 3) 1)))))
                          (resume (+ (* (* late (+ (% n 3) 1)) 100) (* (+ avail 1) 10))
                                  (tuple (+ avail 1) holds (+ fines (* late (+ (% n 3) 1)))))))))
                 (audit () st
                  (match st ((tuple avail holds fines) (resume (+ (* fines 100) (+ (* avail 10) holds)) st)))))
                (let ((a (B.borrow)))
                  (let ((b (B.borrow)))
                    (let ((c (B.borrow)))
                      (let ((d (B.ret (: 2 Int64))))
                        (let ((e (B.ret (: 0 Int64))))
                          (let ((f (B.audit)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11001012490010410 Int64))
  (call   main (: 0 Int64)) (output (: 11001012290010210 Int64)))
