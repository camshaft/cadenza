(case "tp1 TWO ops each own a tuple-state SLOT — lo advances field 0, hi doubles field 1, interleaved"
  (input  (do
            (effect Tw (op lo (-> Int64)) (op hi (-> Int64)))
            (def (main (: n Int64))
              (handle Tw (tuple n (* n 10))
                ((lo () s (resume (. s 0) (tuple (+ (. s 0) 1) (. s 1))))
                 (hi () s (resume (. s 1) (tuple (. s 0) (* (. s 1) 2)))))
                (+ (Tw.lo) (+ (Tw.hi) (+ (Tw.lo) (Tw.hi))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 161 Int64))
  (call   main (: 1 Int64)) (output (: 33 Int64)))
