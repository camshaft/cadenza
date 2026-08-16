(case "oi2c Map VALUE-only face: grounding value second"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: k Int64))
          (Map.len (Map.insert (Map.insert Map.empty 1 (Nil unit)) 2 (Full k))))
        (export main)))
  (call   main (: 4 Int64)) (output (: 2 Int64)))
