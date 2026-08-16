(case "gm1 a user-generic sum instantiated at a RECORD payload keys a Map by deep content"
  (input  (do
            (type (Box a) (Full a) (Nil unit))
            (def (main (: n Int64))
              (match (Map.lookup (Map.insert Map.empty (Full (record (x 1) (y 2))) 42)
                                 (Full (record (x n) (y 2))))
                ((Some v) v) ((None _u) -1)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 42 Int64))
  (call   main (: 9 Int64)) (output (: -1 Int64)))
