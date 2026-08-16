(case "cf2 a trie-capturing closure stored IN a list is extracted and applied (registry over a trie)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 6)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def fs (list (fn ((: k Int64)) (match (Map.lookup m k) ((Some v) v) ((None _u) -1)))
                              (fn ((: k Int64)) (* k 100))))
                (match (List.at fs 0)
                  ((Some f) (f 30))
                  ((None _u) -2))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 180 Int64)))
