(case "rs2 the arm updates the record state via Record.with — field b is held by the functional update across dispatches"
  (input  (do
            (effect R (op bump (-> Int64)))
            (def (main (: n Int64))
              (handle R (record (a n) (b 100))
                ((bump () s (resume (+ (. s a) (. s b)) (Record.with s #"a" (+ (. s a) 1)))))
                (+ (R.bump) (+ (R.bump) (R.bump)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 318 Int64))
  (call   main (: 0 Int64)) (output (: 303 Int64)))
