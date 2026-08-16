(case "hs1 a handler SEED built by a 40-key trie fill enumerates from the arm"
  (input  (do
            (effect Rd (op keys (-> Unit Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
            (def (main (: n Int64))
              (handle Rd (fill n Map.empty)
                ((keys (u) s (resume (Map.len s) s)))
                (+ (Rd.keys) (Rd.keys))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 80 Int64)))
