(case "g2 a failed guard re-decodes the NEXT arm's binder from the same segment cleanly"
  (input  (do
            (def (main (: k UInt8))
              (match (Bytes.of (list (UInt8.wrap 5) (UInt8.wrap k) (UInt8.wrap (+ k 1))))
                ((guard (bin (u8 5) (u8 n) (u8 p)) (> n 50)) (+ n p))
                ((guard (bin (u8 5) (u8 a) (u8 b)) (> b a)) (* 10 (+ a b)))
                ((bin (u8 5) (u8 x) (u8 y)) (* 100 (+ x y)))
                (_ -1)))
            (export main)))
  (call   main (: 60 UInt8)) (output (: 121 Int64))
  (call   main (: 7 UInt8)) (output (: 150 Int64)))
