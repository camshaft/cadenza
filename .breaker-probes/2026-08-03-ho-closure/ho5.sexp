(case "ho5 inner closure applied to its OWN result through the same outer handle"
  (input  (do (def (mk) (fn ((: f (-> Int64 Int64))) (f (f 3))))
              (def (app (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (g (fn (y) (+ y x))))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: 13 Int64)))
