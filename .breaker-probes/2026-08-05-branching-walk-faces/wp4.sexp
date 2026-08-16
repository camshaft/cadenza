(case "wp4 an ABORT fires MID-WALK: left subtree visited, abort in right discards the rest"
  (input  (do
            (type Exp (Lit Int64) (Add Exp Exp))
            (effect Cnt (op bump (-> Unit Int64)) (op halt (-> Unit Int64)))
            (def (walk (: e Exp))
              (match e
                ((Exp.Lit v) (if (= v 99) (Cnt.halt) (+ (Cnt.bump) (* 0 v))))
                ((Exp.Add a b) (+ (walk a) (walk b)))))
            (def (main (: n Int64))
              (handle Cnt 0
                ((bump (u) s (resume s (+ s 1)))
                 (halt (u) s (* 1000 s)))
                (walk (Exp.Add (Exp.Add (Exp.Lit n) (Exp.Lit n)) (Exp.Add (Exp.Lit 99) (Exp.Lit n))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 2000 Int64)))
