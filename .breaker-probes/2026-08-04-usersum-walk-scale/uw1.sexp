(case "uw1 a user-sum expression tree evaluated recursively at depth 500 (the compiler-pass walk at scale)"
  (input  (do
            (type Exp (Lit Int64) (Add Exp Exp))
            (def (build (: i Int64) (: e Exp))
              (if (= i 0) e (build (- i 1) (Exp.Add e (Exp.Lit 1)))))
            (def (eval-exp (: e Exp))
              (match e
                ((Exp.Lit v) v)
                ((Exp.Add a b) (+ (eval-exp a) (eval-exp b)))))
            (def (main (: n Int64))
              (eval-exp (build n (Exp.Lit 5))))
            (export main)))
  (call   main (: 500 Int64)) (output (: 505 Int64)))
