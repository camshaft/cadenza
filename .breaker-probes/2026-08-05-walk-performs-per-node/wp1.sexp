(case "wp1 a recursive tree WALK performs an effect per node; handler state counts 200 visits"
  (input  (do
            (type Exp (Lit Int64) (Add Exp Exp))
            (effect Cnt (op bump (-> Unit Int64)))
            (def (build (: i Int64) (: e Exp))
              (if (= i 0) e (build (- i 1) (Exp.Add e (Exp.Lit 1)))))
            (def (walk (: e Exp))
              (match e
                ((Exp.Lit v) (do (def _b (Cnt.bump)) v))
                ((Exp.Add a b) (+ (walk a) (walk b)))))
            (def (main (: n Int64))
              (handle Cnt 0
                ((bump (u) s (resume s (+ s 1))))
                (do
                  (def total (walk (build n (Exp.Lit 5))))
                  (+ (* 10 (Cnt.bump)) total))))
            (export main)))
  (call   main (: 199 Int64)) (output (: 2204 Int64)))
