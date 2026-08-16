(case "n4 runtime UInt8 bit-xor with all-ones stays in width"
  (input  (do
            (def (main (: x UInt8))
              (Int64.of (^ x (UInt8.wrap 255))))
            (export main)))
  (call   main (: 0 UInt8)) (output (: 255 Int64))
  (call   main (: 200 UInt8)) (output (: 55 Int64)))
