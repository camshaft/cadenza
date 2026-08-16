(case "oi2b Map KEY-only face: grounding key second"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: k Int64))
          (Map.len (Map.insert (Map.insert Map.empty (Nil unit) 1) (Full k) 2)))
        (export main)))
  (call   main (: 4 Int64)) (output (: 2 Int64)))
