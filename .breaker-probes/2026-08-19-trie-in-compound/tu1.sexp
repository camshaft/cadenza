(case "tu1 a trie stored inside a TUPLE component reads through the projection at depth"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 8)))))
            (def (main (: n Int64))
              (do
                (def pair (tuple (fill n Map.empty) "meta"))
                (+ (* 10 (match (Map.lookup (. pair 0) 30) ((Some v) v) ((None _u) -1)))
                   (String.byte-len (. pair 1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 2404 Int64)))
