(case "h8e get THEN ping (value op first)"
  (input  (do
            (effect io (op ping (-> Int64 Unit)) (op get (-> Int64 Int64)))
            (def (main (: k Int64))
              (host (io)
                (let ((v (io.get k)))
                  (do (io.ping v) (+ v k)))))
            (export main)))
  (host-responses (respond io.get (: 7 Int64)) (respond io.ping (: 0 Int64)))
  (host-calls (call io.get) (call io.ping))
  (call   main (: 3 Int64)) (output (: 10 Int64)))
