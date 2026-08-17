(case "xhsGGrow conditional mid-arm foreign perform WITH a growing List.push next-state — composition of the conditional-selector freeze (7106ad497) and the growing-state correct-fold (95f5ab8d2)"
  (input  (do
            (effect O (op note (-> Int64 Int64)))
            (effect I (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle O (: 0 Int64)
                ((note (v) acc
                  (resume (+ acc v) (+ acc v))))
                (handle I (list)
                  ((step (x) col
                    (let ((c2 (+ (List.len col) (+ x (% n 3)))))
                      (if (> c2 5)
                          (let ((nv (O.note c2)))
                            (resume (+ (* c2 10) (% nv 10)) (List.push col c2)))
                          (resume (* c2 10) (List.push col c2))))))
                  (let ((p (I.step (: 3 Int64))))
                    (let ((q (I.step (: 5 Int64))))
                      (let ((r (O.note (: 100 Int64))))
                        (+ (* 1000 (+ (* 1000 p) q)) r)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 40077107 Int64))
  (call   main (: 0 Int64)) (output (: 30066106 Int64)))
