(case "eh1 a handler over a recursive fold that GROWS a trie 60 deep then hands it out whole"
  (input  (do
            (effect Bld (op grow (-> Int64 Int64)) (op take (-> Unit (Map Int64 Int64))))
            (def (feed (: i Int64) (: n Int64))
              (if (= i n) 0 (+ (Bld.grow i) (feed (+ i 1) n))))
            (def (main (: n Int64))
              (do
                (def m (handle Bld Map.empty
                         ((grow (v) s (resume 0 (Map.insert s v (* v 2))))
                          (take (u) s (resume s s)))
                         (do
                           (feed 1 (+ n 1))
                           (Bld.take))))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m 45) ((Some v) (if (= v 90) 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 60 Int64)) (output (: 601 Int64)))
