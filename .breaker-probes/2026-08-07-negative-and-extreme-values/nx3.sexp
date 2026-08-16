(case "nx3 the arm branches on the op arg's SIGN — negation on the negative path, n and -n both exercised"
  (input  (do
            (effect St (op push (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((push (v) s (if (< v 0) (resume (- 0 v) (- s 1)) (resume v (+ s 1)))))
                (+ (St.push n) (+ (* 10 (St.push (- 0 n))) (* 100 (St.push -3))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 355 Int64))
  (call   main (: -2 Int64)) (output (: 322 Int64))
  (call   main (: 0 Int64)) (output (: 300 Int64)))
