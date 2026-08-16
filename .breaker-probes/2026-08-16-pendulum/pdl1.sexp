(case "pdl1 a PENDULUM with friction — each swing hands the active quantity across losing one (the phase field tracks which side holds it), a dead pendulum answers -1 forever, and the tall drop is still swinging at the sixth swing while the short one dies at the fifth with the zero-crossing row pinning the exact stop"
  (input  (do
            (effect P (op swing (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (+ n 4) (: 0 Int64))
                ((swing () st
                  (match st
                    ((tuple active phase)
                      (if (= active 0)
                          (resume -1 st)
                          (resume (- active 1) (tuple (- active 1) (- 1 phase))))))))
                (let ((a (P.swing)))
                  (let ((b (P.swing)))
                    (let ((c (P.swing)))
                      (let ((d (P.swing)))
                        (let ((e (P.swing)))
                          (let ((f (P.swing)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 131211100908 Int64))
  (call   main (: 0 Int64)) (output (: 30200999899 Int64)))
