(case "oi2 Map with the grounding KEY last and the grounding VALUE first (independent solves)"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: k Int64))
          (Map.len (Map.insert (Map.insert Map.empty (Nil unit) (Full k)) (Full k) (Nil unit))))
        (export main)))
  (call   main (: 4 Int64)) (output (: 2 Int64)))
