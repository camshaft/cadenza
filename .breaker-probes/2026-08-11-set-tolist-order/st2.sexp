(case "st2 the ascending drain holds across NEGATIVES and zero — signed ordering of the set survives the state thread"
  (input  (do
            (effect S (op add (-> Int64 Int64)) (op drain (-> Int64)))
            (def (fold-l (: xs (List Int64)) (: acc Int64))
              (match xs
                ((list h .. t) (fold-l t (+ (* 100 acc) (+ h 50))))
                (_other acc)))
            (def (main (: n Int64))
              (handle S (Set.of (list))
                ((add (v) s (resume (Set.len s) (Set.insert s v)))
                 (drain () s (resume (fold-l (Set.to-list s) 0) s)))
                (let ((_a (S.add -3)))
                  (let ((_b (S.add 0)))
                    (let ((_c (S.add n)))
                      (S.drain))))))
            (export main)))
  (call   main (: 9 Int64)) (output (: 475059 Int64))
  (call   main (: -8 Int64)) (output (: 424750 Int64)))
