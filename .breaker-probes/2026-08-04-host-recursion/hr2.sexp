(case "hr2 a host call in a NON-TAIL position accumulates through the unwind (post-recursion reads)"
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (def (walk (: n Int64))
              (if (> n 0) (+ (* 10 (walk (- n 1))) (io.get)) 0))
            (def (main (: n Int64))
              (host (io) (walk n)))
            (export main)))
  (host-responses (respond io.get (: 3 Int64))
                  (respond io.get (: 7 Int64))
                  (respond io.get (: 5 Int64)))
  (host-calls (call io.get) (call io.get) (call io.get))
  (call   main (: 3 Int64))
  (output (: 375 Int64)))
