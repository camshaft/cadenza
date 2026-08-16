(case "tl2 the to-list output WALKED with a per-element perform (order × effect composition)"
  (input  (do
            (effect St (op a (-> Unit Int64)) (op w (-> Int64 Int64)))
            (def (walk (: xs (List Int64)) (: acc Int64))
              (match xs
                ((list) acc)
                ((list h .. t) (walk t (+ (* acc 10) (St.w h))))))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume s (- s 2)))
                 (w (v) s (resume v s)))
                (walk (Set.to-list (Set.insert (Set.insert (Set.of (list)) (St.a)) (St.a))) 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 35 Int64)))
