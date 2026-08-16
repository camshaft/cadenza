(case "cmmin3 SCALAR arg, SUM state - two dispatches (Idle->Run->Run accumulate)"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (effect M (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (k) s
                  (match s
                    ((Mode.Idle) (resume k (Mode.Run k)))
                    ((Mode.Run j) (resume (+ j k) (Mode.Run (+ j k)))))))
                (+ (M.step (+ 10 n))
                   (M.step 7))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 37 Int64)))
