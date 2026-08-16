(case "f2 const f32: the SAME 0.1+0.2 = 0.3 comparison const-folded must agree with the runtime answer (1)"
  (input  (do
            (def (main)
              (if (= (+ (: 0.1 Float32) (: 0.2 Float32)) (: 0.3 Float32)) 1 0))
            (export main)))
  (call   main) (output (: 1 Int64)))
