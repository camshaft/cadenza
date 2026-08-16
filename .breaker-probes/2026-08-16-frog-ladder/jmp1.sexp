(case "jmp1 a TIRING frog on a ladder — each jump advances by the current stride which then SHRINKS by one bottoming at one, rest restores the seed stride answering the distance so far, and the longer opening stride tires through a different lattice (five-four-three versus three-two-one) so the gap between the frogs WIDENS at every row"
  (input  (do
            (effect J
              (op jump (-> Int64))
              (op rest (-> Int64)))
            (def (main (: n Int64))
              (handle J (tuple (: 0 Int64) (+ (% n 4) 3))
                ((jump () st
                  (match st
                    ((tuple pos stride)
                      (if (< 1 stride)
                          (resume (+ pos stride) (tuple (+ pos stride) (- stride 1)))
                          (resume (+ pos 1) (tuple (+ pos 1) 1))))))
                 (rest () st
                  (match st
                    ((tuple pos stride) (resume pos (tuple pos (+ (% n 4) 3)))))))
                (let ((a (J.jump)))
                  (let ((b (J.jump)))
                    (let ((c (J.jump)))
                      (let ((d (J.rest)))
                        (let ((e (J.jump)))
                          (let ((f (J.jump)))
                            (let ((g (J.rest)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5091212172121 Int64))
  (call   main (: 0 Int64)) (output (: 3050606091111 Int64)))
