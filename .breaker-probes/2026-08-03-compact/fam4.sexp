(case "fam4 compact read twice with BOTH reads as order-compares (no eq)"
  (input  (do
            (def (build-rope (: n Int64) (: acc Bytes))
              (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap 65))))) acc))
            (def (main (: n Int64))
              (let ((rope (build-rope n (Bytes.of (list)))))
                (let ((flat (Bytes.compact rope)))
                  (+ (if (< flat (Bytes.concat rope (Bytes.of (list (UInt8.wrap 66))))) 1 0)
                     (* 10 (if (< (Bytes.of (list)) flat) 1 0))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11 Int64)))
