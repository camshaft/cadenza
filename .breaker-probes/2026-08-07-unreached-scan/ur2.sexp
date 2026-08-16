(case "ur2 a REACHABLE escaping closure beside a dead effectful one still rejects CDZ0406"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main (: k Int64))
              (host (ask)
                (if (> k 100)
                    (tuple 0 (fn ((: x Int64)) x))
                    (tuple (if true 1 ((fn ((: y Int64)) (+ y (ask.ask))) 0))
                           (fn ((: x Int64)) (+ x (ask.ask)))))))
            (export main)))
  (error  CDZ0406))
