(case "hr4 a host-call COUNT below the supplied responses is graded (extra rows unconsumed)"
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (host (io) (if (> n 5) (+ (io.get) (io.get)) (io.get))))
            (export main)))
  (host-responses (respond io.get (: 3 Int64))
                  (respond io.get (: 7 Int64)))
  (host-calls (call io.get))
  (call   main (: 3 Int64))
  (output (: 3 Int64)))
