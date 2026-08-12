(case "pfxmin5 minimize: three adds, LITERAL-arg add only (no computed (+ n 1))"
  (input  (do
            (effect S
              (op add (-> Int64 Int64))
              (op range (-> Int64 Int64 Int64)))
            (def (last (: xs (List Int64)))
              (match (List.at xs (- (List.len xs) 1)) ((Some v) v) ((None u) 0)))
            (def (main (: n Int64))
              (handle S (list 0)
                ((add (v) pre
                  (let ((t (+ (last pre) v)))
                    (resume t (List.push pre t))))
                 (range (i j) pre
                  (resume (match (List.at pre i)
                            ((Some a) (match (List.at pre j)
                                        ((Some b) (- b a))
                                        ((None u) -1)))
                            ((None u) -1))
                          pre)))
                (let ((_a (S.add n)))
                  (let ((_b (S.add 4)))
                    (let ((_c (S.add 9)))
                      (S.range 0 3))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 16 Int64)))
