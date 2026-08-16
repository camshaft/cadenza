(case "h3 host-delegation face: does the delegated path show it too"
  (input  (do
            (effect io (op ping (-> Unit Int64)))
            (def (main (: x Int64))
              (host (io)
                (let ((v (let ((b true)) (if b (io.ping) 99))))
                  (+ (* 10 v) (io.ping)))))
            (export main)))
  (call   main (: 3 Int64))
  (host-responses (respond io.ping (: 5 Int64)) (respond io.ping (: 6 Int64)))
  (output (: 56 Int64)))
