(case "ah-x2 the do-spine control: pre-abort host call on the spine IS issued (pinned behavior)"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (host (ask)
                (handle Bail 0
                  ((bail (n) s n))
                  (do (ask.ask) (+ 999 (Bail.bail 7))))))
            (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 7 Int64)))
