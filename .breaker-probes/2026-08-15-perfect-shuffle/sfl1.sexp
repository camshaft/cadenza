(case "sfl1 a PERFECT-SHUFFLE position tracker — each shuffle doubles the tracked card's position mod seven (the out-shuffle orbit on eight cards, position seven fixed), where packs position and shuffle count, and the two seeds ride the SAME 3-cycle orbit entered at different points so the rows are rotations of each other"
  (input  (do
            (effect S
              (op shuffle (-> Int64))
              (op where (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple (+ (% n 7) 1) (: 0 Int64))
                ((shuffle () st
                  (match st
                    ((tuple pos count)
                      (if (= pos 7)
                          (resume 7 (tuple 7 (+ count 1)))
                          (resume (% (* 2 pos) 7) (tuple (% (* 2 pos) 7) (+ count 1)))))))
                 (where () st
                  (match st
                    ((tuple pos count) (resume (+ (* pos 10) count) st)))))
                (let ((a (S.shuffle)))
                  (let ((b (S.shuffle)))
                    (let ((c (S.where)))
                      (let ((d (S.shuffle)))
                        (let ((e (S.shuffle)))
                          (let ((f (S.shuffle)))
                            (let ((g (S.where)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1022204010225 Int64))
  (call   main (: 0 Int64)) (output (: 2044201020445 Int64)))
