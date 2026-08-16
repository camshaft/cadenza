(case "fam6 compact eq-then-CONCAT (adv-66 exact second-read op, first read eq vs rope)"
  (input  (do
            (def (build-rope (: n Int64) (: acc Bytes))
              (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap 65))))) acc))
            (def (main (: n Int64))
              (let ((rope (build-rope n (Bytes.of (list)))))
                (let ((flat (Bytes.compact rope)))
                  (+ (if (= rope flat) 1 0)
                     (* 10 (Bytes.len (Bytes.concat flat (Bytes.of (list (UInt8.wrap 66))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 111 Int64)))
