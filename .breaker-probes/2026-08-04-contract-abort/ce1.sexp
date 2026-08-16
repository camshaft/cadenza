(case "ce1 an abortive perform in a contracted body skips the @ensures (no post to check)"
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (@ (ensures (< ret 10)) (def (f (: x Int64))
              (if (< x 0) (Bail.bail 99) (+ x 1))))
            (def (main (: x Int64))
              (handle Bail 0 ((bail (n) s n)) (f x)))
            (export main)))
  (call   main (: -3 Int64)) (output (: 99 Int64))
  (call   main (: 5 Int64)) (output (: 6 Int64)))
