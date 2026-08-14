(case "lhn1 a LUHN checksum accumulator — feed doubles every second digit by POSITION PARITY subtracting nine from two-digit doubles, chk answers 1 on a multiple of ten else the residue, and the seed-fed digit doubles on both runs but only one crosses the subtract-nine threshold"
  (input  (do
            (effect L
              (op feed (-> Int64 Int64))
              (op chk (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (: 0 Int64) (: 0 Int64))
                ((feed (d) st
                  (match st
                    ((tuple s pos)
                      (if (= (% pos 2) 1)
                          (if (< 9 (* d 2))
                              (resume (+ s (- (* d 2) 9)) (tuple (+ s (- (* d 2) 9)) (+ pos 1)))
                              (resume (+ s (* d 2)) (tuple (+ s (* d 2)) (+ pos 1))))
                          (resume (+ s d) (tuple (+ s d) (+ pos 1)))))))
                 (chk () st
                  (match st
                    ((tuple s pos)
                      (if (= (% s 10) 0)
                          (resume 1 st)
                          (resume (% s 10) st))))))
                (let ((a (L.feed 4)))
                  (let ((b (L.feed (% n 7))))
                    (let ((c (L.feed 9)))
                      (let ((d (L.feed 7)))
                        (let ((e (L.chk)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 410192404 Int64))
  (call   main (: 0 Int64)) (output (: 404131808 Int64)))
