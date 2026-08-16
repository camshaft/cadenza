(case "hc3 an ABORT arm's value derives from a HOST op performed in the abort path"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect St (op halt (-> Unit Int64)))
            (def (main)
              (host (ask)
                (handle St 3
                  ((halt (u) s (* s (ask.ask))))
                  (+ 500 (St.halt)))))
            (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 300 Int64)))
