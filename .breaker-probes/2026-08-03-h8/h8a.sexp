(case "h8a a unit-result host op fires IN ORDER between value-bearing ops"
  (input  (do
            (effect io (op log (-> Int64 Unit)) (op get (-> Unit Int64)))
            (def (main (: k Int64))
              (host (io)
                (do (io.log k)
                    (let ((v (io.get unit)))
                      (do (io.log v)
                          (+ v k))))))
            (export main)))
  (host-responses (respond io.log (: 0 Int64)) (respond io.get (: 7 Int64)) (respond io.log (: 0 Int64)))
  (host-calls (call io.log) (call io.get) (call io.log))
  (call   main (: 3 Int64)) (output (: 10 Int64)))
