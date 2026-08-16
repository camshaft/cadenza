(case "gk2 a MIXED-variant sum keys a trie: discriminant-first descent at depth"
  (input  (do
            (type Tag (Lo Int64) (Hi Int64))
            (def (fill (: i Int64) (: m (Map Tag Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (if (= (% i 2) 0) (Tag.Lo i) (Tag.Hi i)) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 100 (Map.len m))
                   (+ (* 10 (match (Map.lookup m (Tag.Lo 24)) ((Some v) (if (= v 24) 1 0)) ((None _u) -1)))
                      (match (Map.lookup m (Tag.Hi 24)) ((Some _v) 0) ((None _u) 1))))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 4011 Int64)))
