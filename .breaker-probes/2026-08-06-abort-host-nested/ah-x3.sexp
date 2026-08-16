(case "ah-x3 the LET-INIT host analog: (let ((x (+ (ask.ask) (Bail.bail 7)))) (+ x 1)) — is the host call issued"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (host (ask)
                (handle Bail 0
                  ((bail (n) s n))
                  (let ((x (+ (ask.ask) (Bail.bail 7)))) (+ x 1)))))
            (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 7 Int64)))
