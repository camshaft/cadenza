(case "tp3 a NESTED tuple state ((a b) c) — the arm rebuilds the inner Fibonacci pair and bumps the outer counter per dispatch"
  (input  (do
            (effect Tw (op step (-> Int64)))
            (def (main (: n Int64))
              (handle Tw (tuple (tuple n 1) 100)
                ((step () s (resume (+ (. (. s 0) 0) (. s 1))
                                    (tuple (tuple (+ (. (. s 0) 0) (. (. s 0) 1)) (. (. s 0) 0)) (+ (. s 1) 1)))))
                (+ (Tw.step) (+ (Tw.step) (Tw.step)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 325 Int64))
  (call   main (: 0 Int64)) (output (: 305 Int64)))
