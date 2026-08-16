(case "e3 TWO closures capturing one let-bound host call (host in the exported def), tuple-destructured in-body"
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (def (main (: k Int64))
              (host (io)
                (let ((v (io.get unit)))
                  (match (tuple (fn ((: x Int64)) (+ v x))
                                (fn ((: x Int64)) (* v x)))
                    ((tuple f g) (+ (f k) (* 100 (g k))))))))
            (export main)))
  (host-responses (respond io.get (: 7 Int64)))
  (host-calls (call io.get))
  (call   main (: 3 Int64)) (output (: 2110 Int64)))
