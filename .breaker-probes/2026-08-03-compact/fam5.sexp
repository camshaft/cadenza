(case "fam5 compact eq-then-LEN (the adv-66 shape with a benign second read)"
  (input  (do
            (def (build-rope (: n Int64) (: acc Bytes))
              (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap 65))))) acc))
            (def (main (: n Int64))
              (let ((rope (build-rope n (Bytes.of (list)))))
                (let ((flat (Bytes.compact rope)))
                  (+ (if (= rope flat) 1 0)
                     (* 10 (Bytes.len flat))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 101 Int64)))
