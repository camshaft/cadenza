(case "cz1 COLLATZ-driven dispatch counting — the walk observes each step, the counter state tallies data-dependent iteration counts"
  (input  (do
            (effect S (op obs (-> Int64 Int64)) (op count (-> Int64)))
            (def (collatz (: x Int64) (: k Int64))
              (if (< k 1) x
                (if (= x 1) x
                  (let ((_o (S.obs x)))
                    (collatz (if (= (% x 2) 0) (/ x 2) (+ (* 3 x) 1)) (- k 1))))))
            (def (main (: n Int64))
              (handle S 0
                ((obs (v) c (resume v (+ c 1)))
                 (count () c (resume c c)))
                (let ((r (collatz n 30)))
                  (+ (* 1000 r) (S.count)))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 1008 Int64))
  (call   main (: 7 Int64)) (output (: 1016 Int64))
  (call   main (: 1 Int64)) (output (: 1000 Int64)))
