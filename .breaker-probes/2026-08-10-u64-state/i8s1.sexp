(case "i8s1 an Int8 handler state walked to the TOP of its range under a guard — the +40 stride stops exactly where one more step would overflow"
  (input  (do
            (effect E (op step (-> Int64)))
            (def (walk (: steps Int64))
              (let ((v (E.step)))
                (if (> v 87) (+ (* 1000 steps) v) (walk (+ steps 1)))))
            (def (main (: n Int8))
              (handle E n
                ((step () s
                  (if (> s (: 87 Int8))
                      (resume (Int64.of s) s)
                      (resume (Int64.of (+ s (: 40 Int8))) (+ s (: 40 Int8))))))
                (walk 0)))
            (export main)))
  (call   main (: 0 Int8)) (output (: 2120 Int64))
  (call   main (: -100 Int8)) (output (: 4100 Int64))
  (call   main (: 87 Int8)) (output (: 127 Int64)))
