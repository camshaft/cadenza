(case "ho2 DEPTH-2 higher-order — the closure arg is itself higher-order"
  (input  (do (def (mk) (fn ((: f (-> (-> Int64 Int64) Int64))) (f (fn (z) (* z 2)))))
              (def (app (: g (-> (-> (-> Int64 Int64) Int64) Int64)) (: x Int64))
                (g (fn ((: h (-> Int64 Int64))) (+ (h x) 1))))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: 11 Int64)))
