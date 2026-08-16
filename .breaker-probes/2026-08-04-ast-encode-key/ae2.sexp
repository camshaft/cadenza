(case "ae2 an Ast.Bytes-bearing encode keys the trie: b-payload keys are distinct from Str keys of the same text"
  (input  (do
            (def (main)
              (do
                (def m (Map.insert (Map.insert Map.empty (Ast.encode (Ast.Bytes b"hi")) 100)
                                   (Ast.encode (Ast.Str "hi")) 200))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (Ast.encode (Ast.Bytes b"hi"))) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (output (: 120 Int64)))
