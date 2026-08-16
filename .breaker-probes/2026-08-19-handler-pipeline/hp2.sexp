(case "hp2 a PIPELINE of three handles: each stage's escaped trie seeds the next"
  (input  (do
            (effect S1 (op a1 (-> Int64 Int64)) (op t1 (-> Unit (Map Int64 Int64))))
            (effect S2 (op a2 (-> Int64 Int64)) (op t2 (-> Unit (Map Int64 Int64))))
            (def (main (: n Int64))
              (do
                (def stage1 (handle S1 Map.empty
                              ((a1 (v) s (resume 0 (Map.insert s v v)))
                               (t1 (u) s (resume s s)))
                              (do (S1.a1 1) (S1.a1 2) (S1.t1))))
                (def stage2 (handle S2 stage1
                              ((a2 (v) s (resume 0 (Map.insert s v (* v 10))))
                               (t2 (u) s (resume s s)))
                              (do (S2.a2 3) (S2.t2))))
                (+ (* 100 (Map.len stage2))
                   (+ (match (Map.lookup stage2 1) ((Some v) v) ((None _u) -1))
                      (match (Map.lookup stage2 3) ((Some v) v) ((None _u) -1))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 331 Int64)))
