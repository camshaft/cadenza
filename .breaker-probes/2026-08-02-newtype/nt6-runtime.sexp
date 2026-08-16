(case "nt6 a runtime UInt64 underflow INSIDE a newtype constructor arg traps (the wrap does not launder the check)"
  (input  (do
            (type D (D UInt64))
            (def (main (: k UInt64))
              (match (D.D (- k 5))
                ((D v) (Int64.of v))))
            (export main)))
  (call   main (: 3 UInt64)) (trap "integer overflow")
  (call   main (: 12 UInt64)) (output (: 7 Int64)))
