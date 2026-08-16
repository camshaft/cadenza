(case "rc1 a RECORD handler state — the arm projects both fields and rebuilds with different advances, sums pin two dispatches"
  (input  (do
            (effect E (op snap (-> (Record (: a Int64) (: b Int64)))))
            (def (main (: n Int64))
              (handle E (record (a n) (b 100))
                ((snap () s (resume s (record (a (+ (. s a) 1)) (b (* (. s b) 2))))))
                (let ((r1 (E.snap)))
                  (let ((r2 (E.snap)))
                    (+ (+ (. r1 a) (. r1 b))
                       (* 10 (+ (. r2 a) (. r2 b))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2165 Int64))
  (call   main (: 0 Int64)) (output (: 2110 Int64))
  (call   main (: -3 Int64)) (output (: 2077 Int64)))
