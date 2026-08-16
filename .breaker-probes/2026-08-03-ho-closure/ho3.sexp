(case "ho3 one higher-order producer feeds TWO distinct consumers"
  (input  (do (def (mk) (fn ((: f (-> Int64 Int64))) (f 10)))
              (def (appa (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (g (fn (y) (+ y x))))
              (def (appb (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (g (fn (y) (* y x))))
              (export mk) (export appa) (export appb)))
  (call   appb (: 4 Int64))
  (output (: 40 Int64)))
