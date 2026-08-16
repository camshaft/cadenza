(case "cp1 a THREE-WAY ordering verdict against the walking state — the fixed probe value crosses from above to below as the state passes it"
  (input  (do
            (effect S (op cmp (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S n
                ((cmp (v) s
                  (resume (if (< v s) -1 (if (> v s) 1 0)) (+ s 1))))
                (let ((a (S.cmp 5)))
                  (let ((b (S.cmp 5)))
                    (let ((c (S.cmp 5)))
                      (+ (* 100 a) (+ (* 10 b) c)))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 99 Int64))
  (call   main (: 5 Int64)) (output (: -11 Int64))
  (call   main (: 6 Int64)) (output (: -111 Int64)))
