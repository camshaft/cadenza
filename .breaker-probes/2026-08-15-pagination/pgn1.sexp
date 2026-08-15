(case "pgn1 a PAGINATION cursor over a seed-sized collection — next serves up to the page size answering the count actually served (the LAST page runs short), a drained cursor answers -1, rewind resets answering the pages served, and the smaller collection drains one page EARLIER so its short page and its -1 land at different rows"
  (input  (do
            (effect P
              (op next (-> Int64 Int64))
              (op rewind (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (: 0 Int64) (: 0 Int64))
                ((next (size) st
                  (match st
                    ((tuple cursor pages)
                      (if (< cursor (+ 20 n))
                          (if (< (- (+ 20 n) cursor) size)
                              (resume (- (+ 20 n) cursor)
                                      (tuple (+ 20 n) (+ pages 1)))
                              (resume size (tuple (+ cursor size) (+ pages 1))))
                          (resume -1 st)))))
                 (rewind () st
                  (match st
                    ((tuple cursor pages) (resume pages (tuple 0 pages))))))
                (let ((a (P.next 8)))
                  (let ((b (P.next 8)))
                    (let ((c (P.next 8)))
                      (let ((d (P.next 8)))
                        (let ((e (P.rewind)))
                          (let ((f (P.next 12)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 80808060412 Int64))
  (call   main (: 0 Int64)) (output (: 80803990312 Int64)))
