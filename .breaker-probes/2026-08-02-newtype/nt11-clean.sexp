(case "nt11 runtime UInt64 underflow inside a newtype ctor traps (no .of in the way)"
  (input  (do
            (type D (D UInt64))
            (def (main (: k UInt64))
              (match (D.D (- k 5))
                ((D v) v)))
            (export main)))
  (call   main (: 3 UInt64)) (trap "integer overflow")
  (call   main (: 12 UInt64)) (output (: 7 UInt64)))
