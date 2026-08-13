(case "utf1 the THREADED Bytes state goes UTF-8-invalid mid-run — a lead byte alone flips decode to None, the completion byte restores it and a bad continuation leaves it broken"
  (input  (do
            (effect S
              (op push (-> Int64 Int64))
              (op dec (-> Int64)))
            (def (main (: n Int64))
              (handle S (String.to-bytes "ab")
                ((push (b) bs
                  (let ((b2 (Bytes.concat bs (Bytes.of (list (UInt8.wrap b))))))
                    (resume (Bytes.len b2) b2)))
                 (dec () bs
                  (resume (match (String.from-bytes bs)
                            ((Some w) (+ (* 10 (String.byte-len w)) 1))
                            ((None u) 0))
                          bs)))
                (let ((a (S.dec)))
                  (let ((b (S.push 195)))
                    (let ((c (S.dec)))
                      (let ((d (S.push (if (= n 0) 169 65))))
                        (let ((e (S.dec)))
                          (+ (* 100 (+ (* 10 (+ (* 100 (+ (* 10 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 21300441 Int64))
  (call   main (: 1 Int64)) (output (: 21300400 Int64)))
