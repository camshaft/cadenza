(case "ah-x4 the do-spine host control at let-depth: (let ((x (do (ask.ask) (Bail.bail 7)))) (+ x 1))"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (host (ask)
                (handle Bail 0
                  ((bail (n) s n))
                  (let ((x (do (ask.ask) (Bail.bail 7)))) (+ x 1)))))
            (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 7 Int64)))
