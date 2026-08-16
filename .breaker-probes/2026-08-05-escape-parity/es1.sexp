(case "es1 an escaping closure performing a delegated effect NESTED inside a tuple is rejected"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask) (tuple 1 (fn ((: x Int64)) (+ x (ask.ask))))))
            (export main)))
  (error  CDZ0406))
