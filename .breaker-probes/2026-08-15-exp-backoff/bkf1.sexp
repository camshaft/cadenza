(case "bkf1 EXPONENTIAL BACKOFF with a seed-shaped cap — fail accrues the current delay into the total then doubles it clamped at the cap, ok resets the delay to one answering the accumulated wait, and the higher cap lets one seed keep doubling where the other saturates two steps earlier"
  (input  (do
            (effect B
              (op fail (-> Int64))
              (op ok (-> Int64)))
            (def (main (: n Int64))
              (handle B (tuple (: 1 Int64) (: 0 Int64))
                ((fail () st
                  (match st
                    ((tuple delay total)
                      (if (< (+ n 6) (* delay 2))
                          (resume (+ n 6) (tuple (+ n 6) (+ total delay)))
                          (resume (* delay 2) (tuple (* delay 2) (+ total delay)))))))
                 (ok () st
                  (match st
                    ((tuple delay total) (resume total (tuple 1 total))))))
                (let ((a (B.fail)))
                  (let ((b (B.fail)))
                    (let ((c (B.fail)))
                      (let ((d (B.fail)))
                        (let ((e (B.ok)))
                          (let ((f (B.fail)))
                            (let ((g (B.fail)))
                              (let ((h (B.ok)))
                                (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 204081615020418 Int64))
  (call   main (: 0 Int64)) (output (: 204060613020416 Int64)))
