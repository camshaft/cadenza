(case "pyj2 a RECURSIVE HELPER AS THE TOLL — each frame's post-resume toll digit-sums a thirteen-fold compound of its captured state so the unwind runs a data-dependent recursion per frame, the two frames' digit-sums collapse different products, and the recursion depth itself varies with the seed through the toll argument"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (dsum (: x Int64))
              (if (< x 10) x (+ (% x 10) (dsum (/ x 10)))))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 1)) (dsum (* (+ s 7) 13)))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 35 Int64))
  (call   main (: 0 Int64)) (output (: 25 Int64)))
