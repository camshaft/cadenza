(case "so3 a host response value drives a List.at INDEX into a trie-retrieved list (double indirection)"
  (input  (do
            (effect io (op idx (-> Unit Int64)))
            (def (fill (: i Int64) (: m (Map Int64 (List Int64))))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (list (* i 10) (* i 20) (* i 30))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (host (io)
                  (match (Map.lookup m 7)
                    ((Some xs) (match (List.at xs (io.idx)) ((Some v) v) ((None _u) -2)))
                    ((None _u) -1)))))
            (export main)))
  (call   main (: 20 Int64))
  (host-responses (respond io.idx (: 2 Int64)))
  (output (: 210 Int64)))
