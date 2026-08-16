(case "oi1 the grounding member LAST in a 3-element set (two open before it)"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: k Int64))
          (Set.len (Set.of (list (Nil unit) (Nil unit) (Full k)))))
        (export main)))
  (call   main (: 4 Int64)) (output (: 2 Int64)))
