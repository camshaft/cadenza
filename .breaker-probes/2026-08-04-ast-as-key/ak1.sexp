(case "ak1 Ast NODES as Map keys: structurally equal nodes collide, different node kinds are distinct"
  (input  (do
            (def (main)
              (do
                (def m (Map.insert (Map.insert (Map.insert Map.empty (Ast.Int 5N) 100)
                                               (Ast.Name "5") 200)
                                   (Ast.Int 5N) 300))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (Ast.Int 5N)) ((Some v) (/ v 100)) ((None _u) -1)))))
            (export main)))
  (output (: 23 Int64)))
