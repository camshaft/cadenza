(case "lr2 a head-elements-plus-REST pattern destructures the dispatched list in the arm — rest length and heads cross per dispatch"
  (input  (do
            (effect S (op grab (-> (List Int64) Int64)))
            (def (main (: n Int64))
              (handle S 0
                ((grab (xs) s
                  (match xs
                    ((list a b .. r) (resume (+ (* 100 a) (+ (* 10 b) (List.len r))) s))
                    (_other (resume -1 s)))))
                (+ (S.grab (list 1 2 3 4 5))
                   (* 10000 (S.grab (list n 7))))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 6700123 Int64))
  (call   main (: 0 Int64)) (output (: 700123 Int64)))
