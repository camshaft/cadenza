(case "wp2 tree walk performing per leaf in OPERAND position (no def-in-do): 200 visits at scale"
  (input  (do
            (type Exp (Lit Int64) (Add Exp Exp))
            (effect Cnt (op bump (-> Unit Int64)))
            (def (build (: i Int64) (: e Exp))
              (if (= i 0) e (build (- i 1) (Exp.Add e (Exp.Lit 1)))))
            (def (walk (: e Exp))
              (match e
                ((Exp.Lit v) (+ v (* 0 (Cnt.bump))))
                ((Exp.Add a b) (+ (walk a) (walk b)))))
            (def (main (: n Int64))
              (handle Cnt 0
                ((bump (u) s (resume s (+ s 1))))
                (+ (* 10 (walk (build n (Exp.Lit 5)))) (Cnt.bump))))
            (export main)))
  (call   main (: 199 Int64)) (output (: 2240 Int64)))
