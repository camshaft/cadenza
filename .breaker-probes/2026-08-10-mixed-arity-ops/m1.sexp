(case "m1 one handler with ops at arity 0 1 2 3 — each result feeds the next call's arguments"
  (input  (do
            (effect M
              (op z (-> Int64))
              (op one (-> Int64 Int64))
              (op two (-> Int64 Int64 Int64))
              (op tri (-> Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle M n
                ((z () s (resume s (+ s 1)))
                 (one (a) s (resume (+ a s) (+ s 10)))
                 (two (a b) s (resume (+ (* 2 a) (+ b s)) (+ s 100)))
                 (tri (a b c) s (resume (+ (* 3 a) (+ (* 2 b) (+ c s))) (+ s 1000))))
                (let ((r0 (M.z)))
                  (let ((r1 (M.one r0)))
                    (let ((r2 (M.two r1 r0)))
                      (let ((r3 (M.tri r2 r1 r0)))
                        (+ (* 1000 r3) (+ (* 100 r2) (+ (* 10 r1) (+ r0 (M.z)))))))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 203665 Int64))
  (call   main (: 0 Int64)) (output (: 154421 Int64)))
