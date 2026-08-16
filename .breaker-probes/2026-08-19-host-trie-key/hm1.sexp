(case "hm1 a HOST response value drives a deep-trie lookup as the KEY at 40-entry scale"
  (input  (do
            (effect io (op pick (-> Unit Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 9)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (host (io)
                  (match (Map.lookup m (io.pick)) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64))
  (host-responses (respond io.pick (: 23 Int64)))
  (output (: 207 Int64)))
