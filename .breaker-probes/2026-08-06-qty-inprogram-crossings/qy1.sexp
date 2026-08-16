(case "qy1 an IN-PROGRAM arm resumes a Qty built in the arm — the erased-scalar crossing without a host"
  (input  (do
            (effect Env (op width (-> Int64 (Qty Int64 (Unit.base #"meter")))))
            (def (main (: n Int64))
              (handle Env 0
                ((width (k) s (resume (Qty.of (* k 2) (Unit.base #"meter")) s)))
                (Qty.value (+ (Env.width n) (Env.width 10)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30 Int64)))
