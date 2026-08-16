(case "pf3 a perform-capture through a TUPLE-returning HOF — both results carry the capture"
  (input  (do
            (effect St (op scale (-> Int64 Int64)))
            (def (map2 (: f (-> Int64 Int64)) (: a Int64) (: b Int64)) (tuple (f a) (f b)))
            (def (main (: n Int64))
              (handle St 10
                ((scale (v) s (resume (* v s) s)))
                (let ((k (St.scale n)))
                  (match (map2 (fn ((: x Int64)) (+ x k)) 1 2)
                    ((tuple p q) (+ (* 100 p) q))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5152 Int64)))
