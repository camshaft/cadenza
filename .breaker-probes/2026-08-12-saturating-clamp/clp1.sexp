(case "clp1 a SATURATING counter — the arm clamps every transition to 0..10 via a pure helper, both bounds hit in one run"
  (input  (do
            (effect S (op nudge (-> Int64 Int64)))
            (def (clamp (: x Int64))
              (if (< x 0) 0 (if (> x 10) 10 x)))
            (def (main (: n Int64))
              (handle S n
                ((nudge (d) s
                  (let ((nx (clamp (+ s d))))
                    (resume nx nx))))
                (let ((a (S.nudge 7)))
                  (let ((b (S.nudge 7)))
                    (let ((c (S.nudge -25)))
                      (+ (* 10000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 71000 Int64))
  (call   main (: 5 Int64)) (output (: 101000 Int64)))
