(case "as6 as-class radius: a HOST perform in the next-state slot ((resume t (+ t (ask.ask))))"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (main (: n Int64))
              (host (ask)
                (handle B n
                  ((step (u) t (resume t (+ t (ask.ask)))))
                  (+ (* 10 (B.step)) (B.step)))))
            (export main)))
  (call   main (: 5 Int64))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 155 Int64)))
