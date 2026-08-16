(case "rs1 a RECORD state where each op owns a FIELD — bump advances a, scale multiplies b, interleaved"
  (input  (do
            (effect R (op bump (-> Int64)) (op scale (-> Int64)))
            (def (main (: n Int64))
              (handle R (record (a n) (b 3))
                ((bump () s (resume (. s a) (record (a (+ (. s a) 1)) (b (. s b)))))
                 (scale () s (resume (. s b) (record (a (. s a)) (b (* (. s b) 10))))))
                (+ (R.bump) (+ (R.scale) (+ (R.bump) (R.scale))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 44 Int64))
  (call   main (: 0 Int64)) (output (: 34 Int64)))
