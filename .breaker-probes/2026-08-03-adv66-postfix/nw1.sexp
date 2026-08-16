(case "nw1 the adv-66 fix's OTHER direction: rope read AFTER its compact is consumed"
  (input  (do
            (def (build-rope (: n Int64) (: acc Bytes))
              (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap 65))))) acc))
            (def (main (: n Int64))
              (let ((rope (build-rope n (Bytes.of (list)))))
                (let ((flat (Bytes.compact rope)))
                  (+ (Bytes.len (Bytes.concat flat (Bytes.of (list (UInt8.wrap 66)))))
                     (* 100 (Bytes.len rope))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1011 Int64))
  (call   main (: 2 Int64)) (output (: 203 Int64)))
