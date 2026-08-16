(case "hr1 a RECURSION-count-driven host-call sequence consumes rows one per iteration in order"
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (def (walk (: n Int64) (: acc Int64))
              (if (> n 0) (walk (- n 1) (+ (* 10 acc) (io.get))) acc))
            (def (main (: n Int64))
              (host (io) (walk n 0)))
            (export main)))
  (host-responses (respond io.get (: 3 Int64))
                  (respond io.get (: 7 Int64))
                  (respond io.get (: 5 Int64)))
  (host-calls (call io.get) (call io.get) (call io.get))
  (call   main (: 3 Int64))
  (output (: 375 Int64)))
