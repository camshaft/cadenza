(case "tc2 NON-tail recursion at depth 2000 with a perform per frame (frame-stack x effects)"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (loop (: n Int64))
              (if (= n 0) 0 (+ (St.a) (loop (- n 1)))))
            (def (main (: k Int64))
              (handle St 0
                ((a (u) s (resume 1 s)))
                (loop k)))
            (export main)))
  (call   main (: 2000 Int64)) (output (: 2000 Int64)))
