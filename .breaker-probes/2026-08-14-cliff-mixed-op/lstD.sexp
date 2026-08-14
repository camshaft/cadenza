(case "lstD a LIST-VALUED dual-use let at four straight-line dispatches — the arm binds the grown list once and uses it for BOTH the resume value and the next state, no branches"
  (input  (do
            (effect L (op grow (-> Int64)))
            (def (lastv (: xs (List Int64)))
              (match (List.at xs (- (List.len xs) 1)) ((Some v) v) ((None u) 0)))
            (def (main (: n Int64))
              (handle L (: (list n 7) (List Int64))
                ((grow () st
                  (let ((r (List.push st (+ (lastv st) 1))))
                    (resume (+ (List.len r) (lastv r)) r))))
                (let ((a (L.grow)))
                  (let ((b (L.grow)))
                    (let ((c (L.grow)))
                      (let ((d (L.grow)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11131517 Int64))
  (call   main (: 0 Int64)) (output (: 11131517 Int64)))
