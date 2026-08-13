(case "cla1 the arm runs a PURE data-dependent-depth recursion on the crossed ARGUMENT — collatz length computed wholly inside one dispatch frame, the state accumulates the lengths and a final read exposes the total"
  (input  (do
            (effect S
              (op probe (-> Int64 Int64))
              (op total (-> Int64)))
            (def (colen (: x Int64) (: k Int64) (: acc Int64))
              (if (< k 1)
                  acc
                  (if (= x 1)
                      acc
                      (colen (if (= (% x 2) 0) (/ x 2) (+ (* 3 x) 1)) (- k 1) (+ acc 1)))))
            (def (main (: n Int64))
              (handle S 0
                ((probe (v) s
                  (let ((c (colen v 64 0)))
                    (resume c (+ s c))))
                 (total () s (resume s s)))
                (let ((a (S.probe n)))
                  (let ((b (S.probe 6)))
                    (let ((c (S.probe 1)))
                      (let ((d (S.total)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 7080015 Int64))
  (call   main (: 27 Int64)) (output (: 64080072 Int64)))
