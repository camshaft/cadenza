(case "px1 a record pattern destructures a trie value INSIDE a handler arm (pattern x arm x retrieval)"
  (input  (do
            (effect Cfg (op read (-> Int64 Int64)))
            (def (fill (: i Int64) (: m (Map Int64 (Record (lo Int64) (hi Int64)))))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (record (lo i) (hi (* i 10)))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (handle Cfg 0
                  ((read (k) s (resume (match (Map.lookup m k)
                                          ((Some r) (match r ((record (lo a) (hi b)) (+ a b))))
                                          ((None _u) -1)) s)))
                  (+ (Cfg.read 5) (Cfg.read 12)))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 187 Int64)))
