(case "z5 a record threaded through recursion with per-step Record.with leaves the seed intact"
  (input  (do
            (def (bump-x (: r (Record (x Int64) (y Int64))))
              (Record.with r #"x" (+ (. r x) 1)))
            (def (go-n (: r (Record (x Int64) (y Int64))) (: n Int64))
              (if (> n 0) (go-n (bump-x r) (- n 1)) r))
            (def (main (: k Int64))
              (let ((seed (record (x k) (y 100))))
                (let ((done (go-n seed 5)))
                  (+ (. done x) (* 1000 (. seed x))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3008 Int64)))
