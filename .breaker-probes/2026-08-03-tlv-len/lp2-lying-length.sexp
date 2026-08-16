(case "lp2 a LYING length prefix (claims 5, payload 3) fails the dependent read to the next arm"
  (input  (do
            (def (main (: k UInt8))
              (let ((framed (bin (u8 7) (u16 5) (bytes (Bytes.of (list k (UInt8.wrap 2) (UInt8.wrap 3)))))))
                (match framed
                  ((bin (u8 7) (u16 n) (bytes body n)) (Int64.of n))
                  (_ -1))))
            (export main)))
  (call   main (: 5 UInt8)) (output (: -1 Int64)))
