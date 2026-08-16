(case "ce1 an escaping closure captures a trie AND the closure re-enters a later handle"
  (input  (do
            (effect Rd (op ask (-> Unit Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 3)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def look (fn ((: k Int64)) (match (Map.lookup m k) ((Some v) v) ((None _u) -1))))
                (handle Rd 12
                  ((ask (u) s (resume s s)))
                  (+ (look (Rd.ask)) (look 5)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 51 Int64)))
