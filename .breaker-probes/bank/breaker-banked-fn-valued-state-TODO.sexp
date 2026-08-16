(case "a handler whose threaded STATE is a closure composes per-perform and applies at the end"
  (input  (do
            (effect Acc (op step (-> Int64 Int64)) (op fin (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Acc (fn ((: v Int64)) v)
                ((step (x) f (resume x (fn ((: v Int64)) (+ (f v) x))))
                 (fin (z) f (resume (f z) f)))
                (do (Acc.step 10)
                    (do (Acc.step n)
                        (Acc.fin 100)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 113 Int64)))
