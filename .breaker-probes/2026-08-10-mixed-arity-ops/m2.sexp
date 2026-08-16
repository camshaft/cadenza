(case "m2 a zero-arg PERFORM inside a three-arg op's argument list — draw ordering within the arg row"
  (input  (do
            (effect M
              (op z (-> Int64))
              (op one (-> Int64 Int64))
              (op tri (-> Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle M n
                ((z () s (resume s (+ s 1)))
                 (one (a) s (resume (+ a s) (+ s 10)))
                 (tri (a b c) s (resume (+ (* 3 a) (+ (* 2 b) (+ c s))) (+ s 1000))))
                (let ((r0 (M.z)))
                  (let ((r1 (M.one r0)))
                    (+ (* 100 (M.tri (M.z) r1 r0)) (M.z))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 7514 Int64))
  (call   main (: 0 Int64)) (output (: 5712 Int64)))
