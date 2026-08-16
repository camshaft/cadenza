(case "mh2 a host response captured BEFORE a multi-shot region is shared by both branches — one host call"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect Amb (op pick (-> Unit Int64)))
            (def (main)
              (host (ask)
                (let ((h (ask.ask)))
                  (handle Amb 0
                    ((pick (u) s (+ (resume 1 s) (resume 2 s))))
                    (+ (* 10 (Amb.pick)) h)))))
            (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 230 Int64)))
