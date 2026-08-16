(case "f1 runtime f32: 0.1+0.2 = 0.3 at Float32 precision (both round to 0x3E99999A)"
  (input  (do
            (def (main (: x Float32))
              (if (= (+ (: 0.1 Float32) (* x (: 0.2 Float32))) (: 0.3 Float32)) 1 0))
            (export main)))
  (call   main (: 1.0 Float32)) (output (: 1 Int64)))
