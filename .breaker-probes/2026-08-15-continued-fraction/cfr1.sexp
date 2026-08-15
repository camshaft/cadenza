(case "cfr1 a CONTINUED-FRACTION expander — each step peels the integer part of p/q answering it then inverts the remainder to (q, p-a*q), a drained fraction answers -1 forever, and the seeds share a denominator but one terminates two steps early so its tail is sentinels while the other is still peeling"
  (input  (do
            (effect F (op step (-> Int64)))
            (def (main (: n Int64))
              (handle F (tuple (+ 100 n) (: 37 Int64))
                ((step () st
                  (match st
                    ((tuple p q)
                      (if (= q 0)
                          (resume -1 st)
                          (resume (/ p q) (tuple q (- p (* (/ p q) q)))))))))
                (let ((a (F.step)))
                  (let ((b (F.step)))
                    (let ((c (F.step)))
                      (let ((d (F.step)))
                        (let ((e (F.step)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 201359899 Int64))
  (call   main (: 0 Int64)) (output (: 201020201 Int64)))
