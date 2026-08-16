(case "cv1 an inner arm resumes with a CHAIN of two outer ops — O.b of O.a evaluated inside the arm, probe pins the double advance"
  (input  (do
            (effect O (op a (-> Int64)) (op b (-> Int64 Int64)) (op probe (-> Int64)))
            (effect I (op ask (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (probe () s (resume s s)))
                (handle I 0
                  ((ask () t (resume (O.b (O.a)) t)))
                  (+ (* 10 (I.ask)) (O.probe)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 67 Int64))
  (call   main (: 0 Int64)) (output (: 25 Int64))
  (call   main (: -5 Int64)) (output (: -80 Int64)))
