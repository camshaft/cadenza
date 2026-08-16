(case "hc2 a HOST op result seeds a handler's initial state ((handle St (ask.ask) ...))"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect St (op get (-> Unit Int64)))
            (def (main)
              (host (ask)
                (handle St (ask.ask)
                  ((get (u) s (resume s (+ s 1))))
                  (+ (St.get) (St.get)))))
            (export main)))
  (host-responses (respond ask.ask (: 50 Int64)))
  (host-calls (call ask.ask))
  (output (: 101 Int64)))
