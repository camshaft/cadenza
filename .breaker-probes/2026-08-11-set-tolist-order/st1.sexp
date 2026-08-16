(case "st1 Set.to-list drains ASCENDING through the arm — insertion order scrambled, the drain's positional fold pins the sort"
  (input  (do
            (effect S (op add (-> Int64 Int64)) (op drain (-> Int64)))
            (def (fold-l (: xs (List Int64)) (: acc Int64))
              (match xs
                ((list h .. t) (fold-l t (+ (* 10 acc) h)))
                (_other acc)))
            (def (main (: n Int64))
              (handle S (Set.of (list))
                ((add (v) s (resume (Set.len s) (Set.insert s v)))
                 (drain () s (resume (fold-l (Set.to-list s) 0) s)))
                (let ((_a (S.add 3)))
                  (let ((_b (S.add 1)))
                    (let ((_c (S.add n)))
                      (S.drain))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 137 Int64))
  (call   main (: 2 Int64)) (output (: 123 Int64)))
