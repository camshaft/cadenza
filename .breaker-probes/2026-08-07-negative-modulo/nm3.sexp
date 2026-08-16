(case "nm3 the only overflowing DIVISION (Int64.min / -1) is a CONSTANT-fold reject — the runtime-divisor form runs for every other divisor"
  (input  (do
            (def (main) (/ -9223372036854775808 -1))
            (export main)))
  (error  CDZ0304))
