(case "nt7 let-bound runtime UInt64 underflow flowing into a newtype ctor traps"
  (input  (do
            (type D (D UInt64))
            (def (main (: k UInt64))
              (let ((delta (- k 5)))
                (match (D.D delta)
                  ((D v) (Int64.of v)))))
            (export main)))
  (call   main (: 3 UInt64)) (trap "integer overflow")
  (call   main (: 12 UInt64)) (output (: 7 Int64)))
