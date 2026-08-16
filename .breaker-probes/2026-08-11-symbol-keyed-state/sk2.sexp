(case "sk2 the SYMBOL key crosses the dispatch as an op argument — interned equality keys the state map from body-side literals"
  (input  (do
            (effect S (op tag (-> Symbol Int64)) (op read (-> Symbol Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((tag (k) s
                  (resume (Map.len s)
                          (Map.insert s k (+ (match (Map.lookup s k) ((Some c) c) ((None _u) n)) 1))))
                 (read (k) s
                  (resume (match (Map.lookup s k) ((Some c) c) ((None _u) -1)) s)))
                (let ((_a (S.tag #"alpha")))
                  (let ((_b (S.tag #"beta")))
                    (let ((_c (S.tag #"alpha")))
                      (+ (* 100 (S.read #"alpha")) (S.read #"beta")))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 201 Int64))
  (call   main (: 5 Int64)) (output (: 706 Int64)))
