(case "hm2 a trie value feeds a host op ARGUMENT and the response re-keys the trie"
  (input  (do
            (effect io (op xform (-> Int64 Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 9)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (host (io)
                  (do
                    (def a (match (Map.lookup m 5) ((Some v) v) ((None _u) -1)))
                    (def r (io.xform a))
                    (match (Map.lookup m r) ((Some v) v) ((None _u) -1))))))
            (export main)))
  (call   main (: 40 Int64))
  (host-responses (respond io.xform (: 12 Int64)))
  (host-calls (call io.xform (: 45 Int64)))
  (output (: 108 Int64)))
