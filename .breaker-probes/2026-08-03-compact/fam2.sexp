(case "fam2 a let-bound Bytes.slice VIEW read twice (eq + order compare)"
  (input  (do
            (def (build-rope (: n Int64) (: acc Bytes))
              (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap 65))))) acc))
            (def (main (: n Int64))
              (let ((rope (build-rope n (Bytes.of (list)))))
                (match (Bytes.slice rope 1 3)
                  ((Some v)
                    (+ (if (= v v) 1 0)
                       (* 10 (if (< v rope) 1 0))))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11 Int64)))
