(case "bh1 a bool host-call ARG crosses and drives the response binding"
  (input  (do
            (effect io (op check (-> Bool Int64)))
            (def (main (: n Int64))
              (host (io) (+ (io.check (> n 5)) (io.check (< n 5)))))
            (export main)))
  (host-responses (respond io.check (: 10 Int64))
                  (respond io.check (: 20 Int64)))
  (host-calls (call io.check) (call io.check))
  (call   main (: 7 Int64))
  (output (: 30 Int64)))
