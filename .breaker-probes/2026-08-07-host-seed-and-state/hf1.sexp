(case "hf1 host calls in the seed AND the next-state slot — the FINAL dispatch's state call is elided"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (host (ask)
                (handle St (ask.ask)
                  ((next (u) s (resume s (+ s (ask.ask)))))
                  (+ (* 10 (St.next)) (St.next)))))
            (export main)))
  (call   main (: 0 Int64))
  (host-responses (respond ask.ask (: 100 Int64)) (respond ask.ask (: 7 Int64)))
  (host-calls (call ask.ask) (call ask.ask))
  (output (: 1107 Int64)))
