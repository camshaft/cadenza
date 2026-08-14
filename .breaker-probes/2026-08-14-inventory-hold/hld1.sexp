(case "hld1 an INVENTORY hold/settle/release protocol — hold reserves against available (on-hand minus already-held) answering the running held total or the NEGATED available on reject, settle deducts held from on-hand, release just drops the holds, and the seeds reject DIFFERENT holds"
  (input  (do
            (effect I
              (op hold (-> Int64 Int64))
              (op settle (-> Int64))
              (op release (-> Int64)))
            (def (main (: n Int64))
              (handle I (tuple (+ n 8) (: 0 Int64))
                ((hold (v) st
                  (match st
                    ((tuple oh hd)
                      (if (< (- oh hd) v)
                          (resume (- 0 (- oh hd)) st)
                          (resume (+ hd v) (tuple oh (+ hd v)))))))
                 (settle () st
                  (match st
                    ((tuple oh hd) (resume (- oh hd) (tuple (- oh hd) 0)))))
                 (release () st
                  (match st
                    ((tuple oh hd) (resume oh (tuple oh 0))))))
                (let ((a (I.hold 4)))
                  (let ((b (I.hold 9)))
                    (let ((c (I.settle)))
                      (let ((d (I.hold 3)))
                        (let ((e (I.release)))
                          (let ((f (I.hold 6)))
                            (let ((g (I.settle)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 4130503049505 Int64))
  (call   main (: 0 Int64)) (output (: 3960403039604 Int64)))
