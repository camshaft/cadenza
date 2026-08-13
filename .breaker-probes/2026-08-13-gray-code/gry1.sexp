(case "gry1 a GRAY-CODE generator — each tick answers the threaded counter's Gray encoding (n XOR n>>1) then advances; the body XOR-popcounts consecutive answers proving the single-bit-change law"
  (input  (do
            (effect S (op tick (-> Int64)))
            (def (pc (: x Int64) (: acc Int64))
              (if (= x 0) acc (pc (>> x 1) (+ acc (& x 1)))))
            (def (main (: n Int64))
              (handle S n
                ((tick () c (resume (^ c (>> c 1)) (+ c 1))))
                (let ((g1 (S.tick)))
                  (let ((g2 (S.tick)))
                    (let ((g3 (S.tick)))
                      (+ (* 100000 g1)
                         (+ (* 10000 g2)
                            (+ (* 1000 g3)
                               (+ (* 10 (pc (^ g1 g2) 0)) (pc (^ g2 g3) 0))))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 267011 Int64))
  (call   main (: 6 Int64)) (output (: 552011 Int64)))
