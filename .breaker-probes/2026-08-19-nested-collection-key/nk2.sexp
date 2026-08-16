(case "nk2 a MAP-valued key trie: the probe map built in reverse insertion order still hits"
  (input  (do
            (def (fill (: i Int64) (: m (Map (Map Int64 Int64) Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (Map.insert (Map.insert Map.empty i 1) (+ i 50) 2) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def probe (Map.insert (Map.insert Map.empty 68 2) 18 1))
                (match (Map.lookup m probe) ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 18 Int64)))
