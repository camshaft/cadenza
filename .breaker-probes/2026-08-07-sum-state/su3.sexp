(case "su3 a RECURSIVE sum state — the arm grows a Peano tower by two per dispatch, a recursive fn measures it"
  (input  (do
            (type Nat (Z) (S Nat))
            (def (depth (: m Nat))
              (match m ((Z) 0) ((S p) (+ 1 (depth p)))))
            (effect T (op grow (-> Int64)))
            (def (main (: n Int64))
              (handle T (Z)
                ((grow () s (resume (depth s) (S (S s)))))
                (+ (T.grow) (+ (* 10 (T.grow)) (* 100 (T.grow))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 420 Int64)))
