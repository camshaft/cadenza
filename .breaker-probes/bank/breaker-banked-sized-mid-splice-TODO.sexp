(case "a sized bytes segment in BUILD position splices a runtime Bytes value mid-form"
  (input  (do
            (def (main (: x UInt8))
              (let ((mid (Bytes.of (list x 20))))
                (match (bin (u8 1) (bytes mid 2) (u8 9))
                  ((bin (u8 a) (u8 b) (u8 c) (u8 d)) (Int64.of (+ (* 1000 a) (+ (* 100 b) (+ (* 10 c) d)))))
                  (_ -1))))
            (export main)))
  (call   main (: 7 UInt8)) (output (: 1729 Int64))
  (call   main (: 0 UInt8)) (output (: 1029 Int64)))

(case "an UNSIZED bytes segment mid-form is rejected as ill-formed"
  (input  (do
            (def (main (: x UInt8))
              (Bytes.len (bin (u8 1) (bytes (Bytes.of (list x 20))) (u8 9))))
            (export main)))
  (call   main (: 7 UInt8))
  (error  CDZ0220))
