(case "ah1 a handler ARM runs a recursive PURE computation on its state before resuming (heavy-arm face)"
  (input  (do
            (effect St (op score (-> Unit Int64)))
            (def (fib (: n Int64))
              (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
            (def (main (: n Int64))
              (handle St n
                ((score (u) s (resume (fib s) (+ s 1))))
                (+ (St.score) (St.score))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 144 Int64)))
