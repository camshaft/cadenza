(case "pf1 a wrapped narrow result feeding a CHECKED add traps at the checked op's own edge"
  (input  (do
            (def (main (: x UInt8))
              (Int64.of (+ (UInt8.wrapping-add x (UInt8.wrap 10)) (UInt8.wrap 6))))
            (export main)))
  (call   main (: 250 UInt8)) (output (: 10 Int64))
  (call   main (: 245 UInt8)) (trap "integer overflow"))
