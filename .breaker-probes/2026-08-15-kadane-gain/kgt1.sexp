(case "kgt1 a KADANE max-subarray tracker — feed extends or RESTARTS the running sum via a four-way comparison lattice answering the live sum, best remembers the peak from a -99 sentinel, and the seed flips the middle feed's sign so one run restarts there while the other extends through it"
  (input  (do
            (effect K
              (op feed (-> Int64 Int64))
              (op bst (-> Int64)))
            (def (main (: n Int64))
              (handle K (tuple (: 0 Int64) (: -99 Int64))
                ((feed (v) st
                  (match st
                    ((tuple cur best)
                      (if (< (+ cur v) v)
                          (if (< best v)
                              (resume v (tuple v v))
                              (resume v (tuple v best)))
                          (if (< best (+ cur v))
                              (resume (+ cur v) (tuple (+ cur v) (+ cur v)))
                              (resume (+ cur v) (tuple (+ cur v) best)))))))
                 (bst () st
                  (match st ((tuple cur best) (resume best st)))))
                (let ((a (K.feed 4)))
                  (let ((b (K.feed -6)))
                    (let ((c (K.feed (- n 5))))
                      (let ((d (K.feed 7)))
                        (let ((e (K.feed -2)))
                          (let ((f (K.bst)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 39805121012 Int64))
  (call   main (: 0 Int64)) (output (: 39795070507 Int64)))
