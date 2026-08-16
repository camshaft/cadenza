(case "oc5 a CLOSURE as the op ARGUMENT — the arm applies the passed strategy to its state"
  (input  (do
            (effect Ap (op app (-> (-> Int64 Int64) Int64)))
            (def (main (: n Int64))
              (handle Ap n
                ((app (f) s (resume (f s) (+ s 1))))
                (+ (* 100 (Ap.app (fn ((: x Int64)) (* x 3)))) (Ap.app (fn ((: x Int64)) (+ x 7))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1513 Int64)))
