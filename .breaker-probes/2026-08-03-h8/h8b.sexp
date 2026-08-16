(case "h8b minimal unit-result host op alone"
  (input  (do
            (effect io (op ping (-> Int64 Unit)))
            (def (main (: k Int64))
              (host (io)
                (do (io.ping k) 42)))
            (export main)))
  (host-responses (respond io.ping (: 0 Int64)))
  (host-calls (call io.ping))
  (call   main (: 3 Int64)) (output (: 42 Int64)))
