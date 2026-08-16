(case "hgl1 an HOURGLASS with flips — tick drains three grains from the top clamped at empty, flip swaps the bulbs answering the new top (total minus old top), and the two totals CONVERGE mid-stream (both read 6,3,0 after the first flip drains) then DIVERGE again at the second flip which restores each glass's own total"
  (input  (do
            (effect H
              (op tick (-> Int64))
              (op flip (-> Int64)))
            (def (main (: n Int64))
              (handle H (tuple (+ 8 n) (+ 8 n))
                ((tick (
                  ) st
                  (match st
                    ((tuple top total)
                      (if (< top 3)
                          (resume 0 (tuple 0 total))
                          (resume (- top 3) (tuple (- top 3) total))))))
                 (flip () st
                  (match st
                    ((tuple top total)
                      (resume (- total top) (tuple (- total top) total))))))
                (let ((a (H.tick)))
                  (let ((b (H.tick)))
                    (let ((c (H.flip)))
                      (let ((d (H.tick)))
                        (let ((e (H.tick)))
                          (let ((f (H.flip)))
                            (let ((g (H.tick)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 15120603001815 Int64))
  (call   main (: 0 Int64)) (output (: 5020603000805 Int64)))
