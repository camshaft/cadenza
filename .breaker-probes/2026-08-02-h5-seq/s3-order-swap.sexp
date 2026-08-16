(case "s3 order swap: host call first, then the discarded pure trap"
  (input  (do
            (effect io (op put (-> Int64 Int64)))
            (def (main (: d Int64))
              (host (io)
                (do (io.put 1)
                    (/ 100 d)
                    42)))
            (export main)))
  (host-responses (respond io.put (: 0 Int64)))
  (host-calls (call io.put))
  (call   main (: 0 Int64)) (output (: 42 Int64)))
