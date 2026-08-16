(case "ut2 unit nested INSIDE a compound payload type (Tuple a unit)"
  (input  (do
        (type (Box a) (Full (Tuple a unit)) (Nil unit))
        (def (main (: k Int64))
          (match (Full (tuple k unit)) ((Full p) (. p 0)) ((Nil _u) -1)))
        (export main)))
  (call   main (: 7 Int64)) (output (: 7 Int64)))
