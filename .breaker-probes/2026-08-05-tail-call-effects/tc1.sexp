(case "tc1 deep TAIL recursion (5000 iters) with a perform per iteration (TCO x effects at depth)"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (loop (: n Int64) (: acc Int64))
              (if (= n 0) acc (loop (- n 1) (+ acc (St.a)))))
            (def (main (: k Int64))
              (handle St 0
                ((a (u) s (resume 1 s)))
                (loop k 0)))
            (export main)))
  (call   main (: 5000 Int64)) (output (: 5000 Int64)))
