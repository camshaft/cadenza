(case "mm1 mutual recursion threading a trie: even/odd builders share one accumulator"
  (input  (do
            (def (even-b (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (odd-b (- i 1) (Map.insert m i (* i 2)))))
            (def (odd-b (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (even-b (- i 1) (Map.insert m i (* i 3)))))
            (def (main (: n Int64))
              (do
                (def m (even-b n Map.empty))
                (+ (* 100 (Map.len m))
                   (+ (match (Map.lookup m 40) ((Some v) (if (= v 80) 1 0)) ((None _u) -1))
                      (match (Map.lookup m 39) ((Some v) (if (= v 117) 10 0)) ((None _u) -1))))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 4011 Int64)))
