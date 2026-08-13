(case "nrs1 a ROVER state — nested record {pos:{x,y}, steps}; each move applies SIGNED deltas to both inner fields via Record.with and answers the manhattan distance"
  (input  (do
            (effect S (op move (-> Int64 Int64 Int64)))
            (def (iabs (: v Int64)) (if (< v 0) (- 0 v) v))
            (def (main (: n Int64))
              (handle S (record (= pos (record (= x n) (= y 0))) (= steps 0))
                ((move (dx dy) s
                  (let ((p2 (Record.with (Record.with (. s pos) #"x" (+ (. (. s pos) x) dx))
                                         #"y" (+ (. (. s pos) y) dy))))
                    (resume (+ (iabs (. p2 x)) (iabs (. p2 y)))
                            (record (= pos p2) (= steps (+ (. s steps) 1)))))))
                (let ((a (S.move 2 3)))
                  (let ((b (S.move -1 4)))
                    (let ((c (S.move 0 -7)))
                      (+ (* 10000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 50801 Int64))
  (call   main (: 5 Int64)) (output (: 101306 Int64)))
