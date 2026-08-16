(case "ho1 higher-order round-trip whose inner closure captures a HEAP list"
  (input  (do (def (mk) (fn ((: f (-> Int64 Int64))) (f 10)))
              (def (app (: g (-> (-> Int64 Int64) Int64)) (: x Int64))
                (let ((xs (list x x x)))
                  (g (fn (y) (+ y (List.len xs))))))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: 13 Int64)))
