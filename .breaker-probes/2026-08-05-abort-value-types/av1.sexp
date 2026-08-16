(case "av1 the abort VALUE is a Map built from the heap state (heap-to-heap abort return)"
  (input  (do
            (effect St (op halt (-> Unit (Map Int64 Int64))))
            (def (main (: a Int64))
              (do
                (def m (handle St (list a (+ a 1))
                         ((halt (u) s (Map.insert Map.empty (List.len s) 42)))
                         (St.halt)))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m 2) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 52 Int64)))
