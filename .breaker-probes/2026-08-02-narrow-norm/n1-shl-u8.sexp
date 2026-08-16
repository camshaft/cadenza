(case "n1 a runtime UInt8 left shift that overflows the width traps (checked shift)"
  (input  (do
            (def (main (: x UInt8))
              (Int64.of (<< x 4)))
            (export main)))
  (call   main (: 20 UInt8)) (trap "integer overflow")
  (call   main (: 3 UInt8)) (output (: 48 Int64)))
