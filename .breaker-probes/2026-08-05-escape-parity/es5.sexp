(case "es5 a LET-bound escaping-effect closure returned from the host block"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask) (let ((f (fn ((: x Int64)) (+ x (ask.ask))))) f)))
            (export main)))
  (error  CDZ0406))
