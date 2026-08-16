(case "fe2 a CLAMP-style Float64 arm (single-site resume) — below-state args are lifted to the rising floor"
  (input  (do
            (effect F (op clip (-> Float64 Float64)))
            (def (main (: n Int64))
              (handle F 0.0
                ((clip (v) s (resume (if (< v s) s v) (+ s 1.0))))
                (let ((a (F.clip (Float64.of-int n))))
                  (let ((b (F.clip -2.5)))
                    (let ((c (F.clip 3.5)))
                      (if (= (+ a (+ b c)) (+ (Float64.of-int n) 4.5)) 7 (if (= (+ a (+ b c)) 8.0) 8 9)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7 Int64))
  (call   main (: -3 Int64)) (output (: 9 Int64))
  (call   main (: 0 Int64)) (output (: 7 Int64)))
