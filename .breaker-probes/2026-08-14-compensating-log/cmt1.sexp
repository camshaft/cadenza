(case "cmt1 a COMPENSATING transaction log — each do applies its delta AND pushes the inverse onto the undo stack, each compensate pops the LAST inverse and applies it, unwinding in strict LIFO order back to the seed"
  (input  (do
            (effect S
              (op dotx (-> Int64 Int64))
              (op comp (-> Int64)))
            (def (lastv (: xs (List Int64)))
              (match (List.at xs (- (List.len xs) 1)) ((Some v) v) ((None u) 0)))
            (def (dropl (: xs (List Int64)) (: i Int64) (: keep Int64) (: acc (List Int64)))
              (if (< i keep)
                  (dropl xs (+ i 1) keep (List.push acc (match (List.at xs i) ((Some v) v) ((None u) 0))))
                  acc))
            (def (main (: n Int64))
              (handle S (tuple n (: (list) (List Int64)))
                ((dotx (v) st
                  (match st
                    ((tuple val undos
                      ) (resume (+ val v) (tuple (+ val v) (List.push undos (- 0 v)))))))
                 (comp () st
                  (match st
                    ((tuple val undos)
                      (if (= (List.len undos) 0)
                          (resume -99 st)
                          (let ((u (lastv undos)))
                            (resume (+ val u)
                                    (tuple (+ val u)
                                           (dropl undos 0 (- (List.len undos) 1) (: (list) (List Int64)))))))))))
                (let ((a (S.dotx 5)))
                  (let ((b (S.dotx 3)))
                    (let ((c (S.comp)))
                      (let ((d (S.dotx 10)))
                        (let ((e (S.comp)))
                          (let ((f (S.comp)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 151815251510 Int64))
  (call   main (: 0 Int64)) (output (: 50805150500 Int64)))
