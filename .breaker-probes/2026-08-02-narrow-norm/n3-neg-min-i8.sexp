(case "n3 negating runtime Int8 MIN traps (two's-complement asymmetry at the narrow width)"
  (input  (do
            (def (main (: x Int8))
              (Int64.of (- (Int8.wrap 0) x)))
            (export main)))
  (call   main (: -128 Int8)) (trap "integer overflow")
  (call   main (: 7 Int8)) (output (: -7 Int64)))
