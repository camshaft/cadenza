(case "so2 a deep trie built BETWEEN two host calls reads correctly after the second"
  (input  (do
            (effect io (op ping (-> Unit Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
            (def (main (: n Int64))
              (host (io)
                (do
                  (def a (io.ping))
                  (def m (fill n Map.empty))
                  (def b (io.ping))
                  (+ (* 1000 (+ a b))
                     (+ (Map.len m)
                        (match (Map.lookup m 42) ((Some v) v) ((None _u) -1)))))))
            (export main)))
  (call   main (: 50 Int64))
  (host-responses (respond io.ping (: 3 Int64)) (respond io.ping (: 4 Int64)))
  (output (: 7092 Int64)))
