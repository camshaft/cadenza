(case "hs1 host-delegated op result feeds an s-around-k arm (E5 x host boundary)"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (host (ask)
                (handle G n
                  ((y (x) s k (+ s (k (+ x (ask.ask))))))
                  (G.y 5))))
            (export main)))
  (call   main (: 100 Int64))
  (host-responses (respond ask.ask (: 3 Int64)))
  (host-calls (call ask.ask))
  (output (: 108 Int64)))
