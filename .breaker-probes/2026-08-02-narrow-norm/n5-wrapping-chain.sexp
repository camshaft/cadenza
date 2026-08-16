(case "n5 a CHAIN of narrow wrapping ops feeds the wrapped intermediate to the next op"
  (input  (do
            (def (main (: x UInt8))
              (Int64.of (UInt8.wrapping-mul (UInt8.wrapping-add x (UInt8.wrap 10)) (UInt8.wrap 2))))
            (export main)))
  (call   main (: 250 UInt8)) (output (: 8 Int64))
  (call   main (: 100 UInt8)) (output (: 220 Int64)))
