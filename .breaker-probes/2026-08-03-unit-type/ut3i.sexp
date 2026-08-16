(case "ut3i same set, Full FIRST"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: k Int64))
          (Set.len (Set.of (list (Full 1) (Nil unit)))))
        (export main)))
  (call   main (: 4 Int64)) (output (: 2 Int64)))
