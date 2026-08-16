(case "w5 runtime UInt8 wrapping-add overflow wraps modulo 256"
  (input  (do
            (def (main (: x UInt8))
              (Int64.of (UInt8.wrapping-add x (UInt8.wrap 10))))
            (export main)))
  (call   main (: 250 UInt8)) (output (: 4 Int64))
  (call   main (: 5 UInt8)) (output (: 15 Int64)))
