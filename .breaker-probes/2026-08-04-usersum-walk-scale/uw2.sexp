(case "uw2 a REWRITE pass over a deep user-sum tree preserves meaning (fold after transform at depth 300)"
  (input  (do
            (type Exp (Lit Int64) (Add Exp Exp))
            (def (build (: i Int64) (: e Exp))
              (if (= i 0) e (build (- i 1) (Exp.Add e (Exp.Lit 1)))))
            (def (double (: e Exp))
              (match e
                ((Exp.Lit v) (Exp.Lit (* 2 v)))
                ((Exp.Add a b) (Exp.Add (double a) (double b)))))
            (def (eval-exp (: e Exp))
              (match e
                ((Exp.Lit v) v)
                ((Exp.Add a b) (+ (eval-exp a) (eval-exp b)))))
            (def (main (: n Int64))
              (- (eval-exp (double (build n (Exp.Lit 5)))) (eval-exp (build n (Exp.Lit 5)))))
            (export main)))
  (call   main (: 300 Int64)) (output (: 305 Int64)))
