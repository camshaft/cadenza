(case "pfxmin minimize: TWO-param op with NESTED Option-match over two List.at reads in the resume value"
  (input  (do
            (effect S (op range (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S (list 10 20 30)
                ((range (i j) pre
                  (resume (match (List.at pre i)
                            ((Some a) (match (List.at pre j)
                                        ((Some b) (- b a))
                                        ((None u) -1)))
                            ((None u) -1))
                          pre)))
                (S.range n (+ n 1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 10 Int64)))
