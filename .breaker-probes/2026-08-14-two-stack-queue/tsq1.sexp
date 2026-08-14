(case "tsq1 a TWO-STACK amortized FIFO queue — enq pushes the in-stack, deq pops the out-stack and reverse-refills from the in-stack only when empty, so an element enqueued between refills never overtakes one already staged"
  (input  (do
            (effect Q
              (op enq (-> Int64 Int64))
              (op deq (-> Int64)))
            (def (lastv (: xs (List Int64)))
              (match (List.at xs (- (List.len xs) 1)) ((Some v) v) ((None u) 0)))
            (def (dropl (: xs (List Int64)) (: i Int64) (: keep Int64) (: acc (List Int64)))
              (if (< i keep)
                  (dropl xs (+ i 1) keep (List.push acc (match (List.at xs i) ((Some v) v) ((None u) 0))))
                  acc))
            (def (rev (: xs (List Int64)) (: i Int64) (: acc (List Int64)))
              (if (< i 0)
                  acc
                  (rev xs (- i 1) (List.push acc (match (List.at xs i) ((Some v) v) ((None u) 0))))))
            (def (main (: n Int64))
              (handle Q (tuple (: (list) (List Int64)) (: (list) (List Int64)))
                ((enq (v) st
                  (match st
                    ((tuple ins outs) (resume v (tuple (List.push ins v) outs)))))
                 (deq () st
                  (match st
                    ((tuple ins outs)
                      (if (= (List.len outs) 0)
                          (let ((r (rev ins (- (List.len ins) 1) (: (list) (List Int64)))))
                            (resume (lastv r) (tuple (: (list) (List Int64)) (dropl r 0 (- (List.len r) 1) (: (list) (List Int64))))))
                          (resume (lastv outs) (tuple ins (dropl outs 0 (- (List.len outs) 1) (: (list) (List Int64))))))))))
                (let ((a (Q.enq (+ n 1))))
                  (let ((b (Q.enq (+ n 2))))
                    (let ((c (Q.deq)))
                      (let ((d (Q.enq (+ n 3))))
                        (let ((e (Q.deq)))
                          (let ((f (Q.deq)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 111211131213 Int64))
  (call   main (: 0 Int64)) (output (: 10201030203 Int64)))
