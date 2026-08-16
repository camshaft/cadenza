(case "ws2 the trie state read through a CONDITION with a performing branch composes"
  (input  (do
            (effect Reg (op put (-> Int64 Int64)) (op len (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Reg Map.empty
                ((put (v) s (resume (Map.len s) (Map.insert s v v)))
                 (len (u) s (resume (Map.len s) s)))
                (do
                  (if (> n 0) (Reg.put 1) 0)
                  (Reg.put 2)
                  (+ (* 10 (Reg.len)) n))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 23 Int64)))
