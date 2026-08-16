(case "hc1 a HOST-delegated op interleaved with a HANDLED op (mixed routing in one body)"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect St (op get (-> Unit Int64)))
            (def (main)
              (host (ask)
                (handle St 5
                  ((get (u) s (resume s (+ s 1))))
                  (+ (St.get) (+ (ask.ask) (St.get))))))
            (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 111 Int64)))
