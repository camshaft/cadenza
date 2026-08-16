(case "wp3 branching walk where the arm ADVANCES using the perform RESULT (accumulating count-of-visits sum)"
  (input  (do
            (type Exp (Lit Int64) (Add Exp Exp))
            (effect Cnt (op bump (-> Unit Int64)))
            (def (build (: i Int64) (: e Exp))
              (if (= i 0) e (build (- i 1) (Exp.Add e (Exp.Lit 1)))))
            (def (walk (: e Exp))
              (match e
                ((Exp.Lit v) (+ (Cnt.bump) (* 0 v)))
                ((Exp.Add a b) (+ (walk a) (walk b)))))
            (def (main (: n Int64))
              (handle Cnt 0
                ((bump (u) s (resume s (+ s 1))))
                (walk (build n (Exp.Lit 5)))))
            (export main)))
  (call   main (: 9 Int64)) (output (: 45 Int64)))
