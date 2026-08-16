(case "g1 a guard cond over a decoded binder that traps must trap, not fall through"
  (input  (do
            (def (main (: k UInt8))
              (match (Bytes.of (list (UInt8.wrap 5) (UInt8.wrap (+ 9 k))))
                ((guard (bin (u8 5) (u8 n)) (> n (/ 12 (- n 9)))) n)
                ((bin (u8 5) (u8 m)) (* 100 m))
                (_ -1)))
            (export main)))
  (call   main (: 0 UInt8)) (trap "division by zero")
  (call   main (: 3 UInt8)) (output (: 12 Int64)))
