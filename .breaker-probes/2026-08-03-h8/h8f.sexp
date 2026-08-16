(case "h8f cursor probe: ping row carries 99 — does get read it?"
  (input  (do
            (effect io (op ping (-> Int64 Unit)) (op get (-> Int64 Int64)))
            (def (main (: k Int64))
              (host (io)
                (do (io.ping k)
                    (+ (io.get k) k))))
            (export main)))
  (host-responses (respond io.ping (: 99 Int64)) (respond io.get (: 7 Int64)))
  (host-calls (call io.ping) (call io.get))
  (call   main (: 3 Int64)) (output (: 10 Int64)))
