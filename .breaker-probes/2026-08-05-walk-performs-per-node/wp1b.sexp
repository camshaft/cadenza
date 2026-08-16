(case "wp1b control: same walk shape, NO effect (pure recursion over the sum)"
  (input  (do
            (type Exp (Lit Int64) (Add Exp Exp))
            (def (build (: i Int64) (: e Exp))
              (if (= i 0) e (build (- i 1) (Exp.Add e (Exp.Lit 1)))))
            (def (walk (: e Exp))
              (match e
                ((Exp.Lit v) v)
                ((Exp.Add a b) (+ (walk a) (walk b)))))
            (def (main (: n Int64))
              (walk (build n (Exp.Lit 5))))
            (export main)))
  (call   main (: 199 Int64)) (output (: 204 Int64)))
