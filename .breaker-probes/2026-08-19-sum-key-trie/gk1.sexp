(case "gk1 a user-GENERIC sum wrapping Int payloads keys a trie at depth"
  (input  (do
            (type (Box a) (Mk a))
            (def (fill (: i Int64) (: m (Map (Box Int64) Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m (Box.Mk i) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (Box.Mk 25)) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 425 Int64)))
