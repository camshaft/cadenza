(case "es4 the peeled-export face: main peels to a plain fn whose body performs the delegated effect"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask) (fn ((: x Int64)) (+ x (ask.ask)))))
            (export main)))
  (error  CDZ0406))
