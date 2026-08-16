(case "m3 mixed-arity draws on a RECURSIVE walk — the two-arg op both consumes and advances state each hop"
  (input  (do
            (effect M
              (op z (-> Int64))
              (op two (-> Int64 Int64 Int64)))
            (def (walk (: k Int64))
              (let ((d (M.two k (M.z))))
                (if (= (% d 7) 0) (+ (* 100 d) k) (walk (+ k 1)))))
            (def (main (: n Int64))
              (handle M n
                ((z () s (resume s (+ s 1)))
                 (two (a b) s (resume (+ (* 2 a) (+ b s)) (+ s 3))))
                (+ (walk 0) (M.z))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 6335 Int64))
  (call   main (: 4 Int64)) (output (: 4928 Int64)))
