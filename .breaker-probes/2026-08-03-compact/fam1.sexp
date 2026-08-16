(case "fam1 a let-bound Bytes.concat result read twice (eq + order)"
  (input  (do
            (def (build-rope (: n Int64) (: acc Bytes))
              (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap 65))))) acc))
            (def (main (: n Int64))
              (let ((rope (build-rope n (Bytes.of (list)))))
                (let ((joined (Bytes.concat rope (Bytes.of (list (UInt8.wrap 66))))))
                  (+ (if (= joined joined) 1 0)
                     (* 10 (if (< rope joined) 1 0))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11 Int64))
  (call   main (: 2 Int64)) (output (: 11 Int64)))
