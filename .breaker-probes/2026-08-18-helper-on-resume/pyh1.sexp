(case "pyh1 the RESUME RESULT FED THROUGH A RECURSIVE PURE HELPER — each arm digit-sums whatever the resumed rest-of-body returned before adding a hundredfold state toll, the helper recursion runs during the unwind on a value that does not exist until the tail completes, and the seed drives the two frames' digit-sums through different collapse depths"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (dsum (: x Int64))
              (if (< x 10) x (+ (% x 10) (dsum (/ x 10)))))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (+ (dsum (resume s (+ s 3))) (* 100 s))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 109 Int64))
  (call   main (: 0 Int64)) (output (: 6 Int64)))
