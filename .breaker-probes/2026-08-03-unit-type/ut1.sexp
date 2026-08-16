(case "ut1 lowercase-unit-payload generic sum as rust Set elements (the #1674 fix face)"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: k Int64))
          (Set.len (Set.of (list (Full k) (Full 2) (Nil unit)))))
        (export main)))
  (call   main (: 2 Int64)) (output (: 2 Int64))
  (call   main (: 5 Int64)) (output (: 3 Int64)))
