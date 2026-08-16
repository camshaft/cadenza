(case "ut3k Map with a Nil-first key set"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: k Int64))
          (match (Map.lookup (Map.insert Map.empty (Nil unit) 42) (Nil unit))
            ((Some v) v) ((None _u) -1)))
        (export main)))
  (call   main (: 4 Int64)) (output (: 42 Int64)))
