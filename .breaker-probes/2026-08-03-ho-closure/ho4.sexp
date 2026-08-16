(case "ho4 the higher-order producer is ALSO applied directly in-guest by a third export"
  (input  (do (def (mk) (fn ((: f (-> Int64 Int64))) (f 10)))
              (def (app (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (g (fn (y) (+ y x))))
              (def (local-use (: x Int64)) ((mk) (fn (y) (* y x))))
              (export mk) (export app) (export local-use)))
  (call   local-use (: 7 Int64))
  (output (: 70 Int64)))
