(case "lstZ the lstQ deq arm mixed with a PASSIVE size reader — rev+dropl double recursion in one branch, second op touches nothing"
  (input  (do
            (effect L (op deq (-> Int64)) (op size (-> Int64)))
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
              (handle L (tuple (: (list n 7) (List Int64)) (: (list) (List Int64)))
                ((deq () st
                  (match st
                    ((tuple ins outs)
                      (if (= (List.len outs) 0)
                          (if (= (List.len ins) 0)
                              (resume -99 st)
                              (let ((r (rev ins (- (List.len ins) 1) (: (list) (List Int64)))))
                                (resume (lastv r) (tuple (: (list) (List Int64)) (dropl r 0 (- (List.len r) 1) (: (list) (List Int64)))))))
                          (resume (lastv outs) (tuple ins (dropl outs 0 (- (List.len outs) 1) (: (list) (List Int64)))))))))
                 (size () st
                  (match st ((tuple ins outs) (resume (+ (List.len ins) (List.len outs)) st)))))
                (let ((a (L.deq)))
                  (let ((b (L.size)))
                    (let ((c (L.deq)))
                      (let ((d (L.size)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 10010700 Int64))
  (call   main (: 0 Int64)) (output (: 10700 Int64)))
