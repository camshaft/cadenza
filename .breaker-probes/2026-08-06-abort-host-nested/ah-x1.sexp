(case "ah-x1 a pre-abort HOST call inside a NESTED strict operand — is the host call still issued"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (host (ask)
                (handle Bail 0
                  ((bail (n) s n))
                  (+ 999 (+ (ask.ask) (Bail.bail 7))))))
            (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 7 Int64)))
