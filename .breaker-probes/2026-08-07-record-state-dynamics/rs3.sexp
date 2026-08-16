(case "rs3 a NESTED record state — the arm functionally updates the inner record's x by y and bumps the outer counter"
  (input  (do
            (effect R (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle R (record (inner (record (x n) (y 2))) (cnt 0))
                ((tick () s (resume (+ (. (. s inner) x) (* 100 (. s cnt)))
                                    (record (inner (Record.with (. s inner) #"x" (+ (. (. s inner) x) (. (. s inner) y))))
                                            (cnt (+ (. s cnt) 1))))))
                (+ (R.tick) (+ (R.tick) (R.tick)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 321 Int64))
  (call   main (: 0 Int64)) (output (: 306 Int64)))
