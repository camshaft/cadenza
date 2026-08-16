(case "s2 control: same shape, non-trapping divisor"
  (input  (do
            (effect io (op put (-> Int64 Int64)))
            (def (main (: d Int64))
              (host (io)
                (do (/ 100 d)
                    (io.put 1)
                    42)))
            (export main)))
  (host-responses (respond io.put (: 0 Int64)))
  (host-calls (call io.put))
  (call   main (: 5 Int64)) (output (: 42 Int64)))
