(case "es6 escalation: the escaping closure nested in a RECORD field rejects CDZ0406"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask) (record (tag 1) (f (fn ((: x Int64)) (+ x (ask.ask)))))))
            (export main)))
  (error  CDZ0406))
(case "es7 escalation: the escaping closure inside a LIST literal rejects CDZ0406"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask) (list (fn ((: x Int64)) (+ x (ask.ask))))))
            (export main)))
  (error  CDZ0406))
