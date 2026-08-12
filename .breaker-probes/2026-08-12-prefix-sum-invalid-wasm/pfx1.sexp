(case "pfx1 a PREFIX-SUM table state — add appends the running total to the list, range answers pre[j]-pre[i] via two fallible reads with an out-of-bounds sentinel"
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
                    (let ((_c (S.add (+ n 1))))
                      (let ((d (S.range 0 3)))
                        (let ((e (S.range 1 3)))
                          (let ((f (S.range 2 9)))
                            (+ (* 10000 d) (+ (* 100 e) (+ f 2)))))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 110801 Int64))
  (call   main (: 10 Int64)) (output (: 251501 Int64)))
