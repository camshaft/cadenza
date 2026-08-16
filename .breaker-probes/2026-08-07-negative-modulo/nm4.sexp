(case "nm4 Int64.min divided by RUNTIME divisors — every non-(-1) divisor has an exact Int64 quotient"
  (input  (do
            (def (main (: d Int64)) (/ -9223372036854775808 d))
            (export main)))
  (call   main (: 2 Int64)) (output (: -4611686018427387904 Int64))
  (call   main (: -2 Int64)) (output (: 4611686018427387904 Int64))
  (call   main (: 1 Int64)) (output (: -9223372036854775808 Int64)))
