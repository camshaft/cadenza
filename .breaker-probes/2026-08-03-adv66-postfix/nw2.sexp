(case "nw2 compact of a compact (idempotent chain) with both intermediates read"
  (input  (do
            (def (build-rope (: n Int64) (: acc Bytes))
              (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap 65))))) acc))
            (def (main (: n Int64))
              (let ((rope (build-rope n (Bytes.of (list)))))
                (let ((flat1 (Bytes.compact rope)))
                  (let ((flat2 (Bytes.compact flat1)))
                    (+ (Bytes.len flat2)
                       (+ (* 100 (if (= flat1 flat2) 1 0))
                          (* 1000 (if (= rope flat2) 1 0))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1110 Int64)))
