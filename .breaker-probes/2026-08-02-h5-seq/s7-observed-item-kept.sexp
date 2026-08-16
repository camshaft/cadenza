(case "s7 a do item whose value feeds the tail via a let is NOT elided (observed trap fires)"
  (input  (do
            (effect io (op put (-> Int64 Int64)))
            (def (main (: d Int64))
              (host (io)
                (let ((q (/ 100 d)))
                  (do (io.put 1)
                      (+ q 2)))))
            (export main)))
  (host-responses (respond io.put (: 0 Int64)))
  (host-calls)
  (call   main (: 0 Int64)) (trap "division by zero"))
