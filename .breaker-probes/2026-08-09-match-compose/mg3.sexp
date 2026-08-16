(case "mg3 match arms returning CLOSURES selected by a runtime sum, applied immediately"
  (input  (do
            (type Op (Add Int64) (Mul Int64))
            (def (main (: k Int64) (: v Int64))
              ((match (if (> k 0) (Add 10) (Mul 3))
                 ((Op.Add n) (fn ((: x Int64)) (+ x n)))
                 ((Op.Mul n) (fn ((: x Int64)) (* x n))))
               v))
            (export main)))
  (call   main (: 1 Int64) (: 5 Int64)) (output (: 15 Int64))
  (call   main (: 0 Int64) (: 5 Int64)) (output (: 15 Int64)))
