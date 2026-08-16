(case "s8 closure-composing-closure over a shared heap capture, source re-read after"
  (input  (do
            (def (main (: k Int64))
              (let ((cap (list k (+ k 10))))
                (let ((f (fn ((: n Int64))
                           (+ n (match (List.at cap 0) ((Some v) v) ((None _u) 0))))))
                  (let ((g (fn ((: n Int64)) (f (f n)))))
                    (+ (g 1) (* 100 (match (List.at cap 1) ((Some v) v) ((None _u) 0))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1511 Int64)))
