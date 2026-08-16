(case "lu2 a RING BUFFER — the tuple state carries a rotating write cursor, the fourth put overwrites slot 0"
  (input  (do
            (effect S (op put (-> Int64 Int64)) (op sum (-> Int64)))
            (def (sum-l (: xs (List Int64)) (: acc Int64))
              (match xs
                ((list h .. t) (sum-l t (+ acc h)))
                (_other acc)))
            (def (main (: n Int64))
              (handle S (tuple (list 0 0 0) 0)
                ((put (v) st
                  (match st
                    ((tuple xs cur)
                      (resume cur (tuple (List.update xs cur v) (% (+ cur 1) 3))))))
                 (sum () st
                  (match st ((tuple xs _c) (resume (sum-l xs 0) st)))))
                (let ((_a (S.put 5)))
                  (let ((_b (S.put 7)))
                    (let ((_c (S.put 11)))
                      (let ((_d (S.put n)))
                        (S.sum)))))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 118 Int64))
  (call   main (: 0 Int64)) (output (: 18 Int64)))
