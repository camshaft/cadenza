(case "bc-gate rope eq-then-order double-read at n=10"
  (input  (do
            (def (build-rope (: n Int64) (: acc Bytes))
              (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap 65))))) acc))
            (def (main (: n Int64))
              (let ((rope (build-rope n (Bytes.of (list)))))
                (let ((flat (Bytes.compact rope)))
                  (+ (if (= rope flat) 1 0)
                     (* 10 (if (< rope (Bytes.concat flat (Bytes.of (list (UInt8.wrap 66))))) 1 0))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11 Int64))
  (call   main (: 2 Int64)) (output (: 11 Int64)))
