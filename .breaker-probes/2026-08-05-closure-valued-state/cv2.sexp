(case "cv2 the arm REPLACES the closure state per perform (strategy evolves: double then increment)"
  (input  (do
            (effect St (op eval (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (fn ((: x Int64)) (* x 2))
                ((eval (v) f (resume (f v) (fn ((: x Int64)) (+ x 1)))))
                (+ (* 100 (St.eval n)) (St.eval 3))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 804 Int64)))
