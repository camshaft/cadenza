(case "ce2 a violated @requires traps BEFORE the body's abortive perform can fire"
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (@ (requires (> x -100)) (def (f (: x Int64))
              (if (< x 0) (Bail.bail 99) (+ x 1))))
            (def (main (: x Int64))
              (handle Bail 0 ((bail (n) s n)) (f x)))
            (export main)))
  (call   main (: -500 Int64)) (trap "unreachable")
  (call   main (: -3 Int64)) (output (: 99 Int64)))
