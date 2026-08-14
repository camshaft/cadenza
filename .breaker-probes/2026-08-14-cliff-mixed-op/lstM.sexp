(case "lstM the lstU program with MIXED dispatches — two enq draws then two deq draws, same arms — tuple-of-two-lists state, the refill branch reverses the in-stack through a recursive def, drained answers -99"
  (input  (do
            (effect L (op deq (-> Int64)) (op enq (-> Int64 Int64)))
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
              (handle L (tuple (: (list) (List Int64)) (: (list) (List Int64)))
                ((enq (v) st
                  (match st
                    ((tuple ins outs) (resume v (tuple (List.push ins v) outs)))))
                 (deq () st
                  (match st
                    ((tuple ins outs)
                      (if (= (List.len outs) 0)
                          (if (= (List.len ins) 0)
                              (resume -99 st)
                              (let ((r (rev ins (- (List.len ins) 1) (: (list) (List Int64)))))
                                (resume (lastv r) (tuple (: (list) (List Int64)) (dropl r 0 (- (List.len r) 1) (: (list) (List Int64)))))))
                          (resume (lastv outs) (tuple ins (dropl outs 0 (- (List.len outs) 1) (: (list) (List Int64))))))))))
                (let ((a (L.enq (+ n 1))))
                  (let ((b (L.enq 7)))
                    (let ((c (L.deq)))
                      (let ((d (L.deq)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11071107 Int64))
  (call   main (: 0 Int64)) (output (: 1070107 Int64)))
