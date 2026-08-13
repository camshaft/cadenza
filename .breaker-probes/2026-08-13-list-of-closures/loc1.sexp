(case "loc1 a LIST-OF-CLOSURES pipeline state — each add pushes a stage function, run folds the input through every staged closure in order, later stages compose after earlier runs"
  (input  (do
            (effect S
              (op addmul (-> Int64 Int64))
              (op addadd (-> Int64 Int64))
              (op run (-> Int64 Int64)))
            (def (fold-run (: fs (List (-> Int64 Int64))) (: i Int64) (: x Int64))
              (match (List.at fs i)
                ((Some f) (fold-run fs (+ i 1) (f x)))
                ((None u) x)))
            (def (main (: n Int64))
              (handle S (: (list) (List (-> Int64 Int64)))
                ((addmul (k) fs
                  (let ((fs2 (List.push fs (fn ((: x Int64)) (* x k)))))
                    (resume (List.len fs2) fs2)))
                 (addadd (k) fs
                  (let ((fs2 (List.push fs (fn ((: x Int64)) (+ x k)))))
                    (resume (List.len fs2) fs2)))
                 (run (v) fs (resume (fold-run fs 0 v) fs)))
                (let ((a (S.addmul 2)))
                  (let ((b (S.addadd n)))
                    (let ((c (S.run 5)))
                      (let ((d (S.addmul 3)))
                        (let ((e (S.run 5)))
                          (+ (* 100 (+ (* 10 (+ (* 100 (+ (* 10 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1213339 Int64))
  (call   main (: 10 Int64)) (output (: 1220360 Int64)))
