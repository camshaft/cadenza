(case "so1 a 60-entry trie SURVIVES a host-delegation round trip as a captured binding"
  (input  (do
            (effect io (op ping (-> Unit Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 3)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (host (io)
                  (+ (* 1000 (io.ping))
                     (+ (* 10 (Map.len m))
                        (match (Map.lookup m 37) ((Some v) (if (= v 111) 1 0)) ((None _u) -1)))))))
            (export main)))
  (call   main (: 60 Int64))
  (host-responses (respond io.ping (: 7 Int64)))
  (output (: 7601 Int64)))
