(case "ut3h set-only with solved element (no match in body)"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: k Int64))
          (Set.len (Set.of (list (Nil unit) (Full 1)))))
        (export main)))
  (call   main (: 4 Int64)) (output (: 2 Int64)))
