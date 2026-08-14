(case "hrn1 a HORNER polynomial evaluator with a MID-STREAM base swap — feed folds acc*x+c answering the running value, swapx replaces the base answering the old one, and the seed shapes the INITIAL base so the pre-swap accumulations diverge while the post-swap coefficients ride on top"
  (input  (do
            (effect H
              (op feed (-> Int64 Int64))
              (op swapx (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle H (tuple (: 0 Int64) (+ (% n 4) 2))
                ((feed (c) st
                  (match st
                    ((tuple acc x)
                      (resume (+ (* acc x) c) (tuple (+ (* acc x) c) x)))))
                 (swapx (v) st
                  (match st
                    ((tuple acc x) (resume x (tuple acc v))))))
                (let ((a (H.feed 3)))
                  (let ((b (H.feed 1)))
                    (let ((c (H.swapx 10)))
                      (let ((d (H.feed 4)))
                        (let ((e (H.feed 2)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 313054742 Int64))
  (call   main (: 0 Int64)) (output (: 307028142 Int64)))
