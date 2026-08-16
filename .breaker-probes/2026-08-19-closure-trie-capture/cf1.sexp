(case "cf1 a closure captures a trie and each application performs a fresh lookup"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 6)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def look (fn ((: k Int64)) (match (Map.lookup m k) ((Some v) v) ((None _u) -1))))
                (+ (* 1000 (look 30))
                   (+ (* 10 (look 5))
                      (if (= (look 999) -1) 1 0)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 180301 Int64)))
