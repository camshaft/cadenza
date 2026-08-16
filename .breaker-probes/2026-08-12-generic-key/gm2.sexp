(case "gm2 nested generic instantiation (Box (Box Int64)) as Set elements dedupes at full depth"
  (input  (do
            (type (Box a) (Full a) (Nil unit))
            (def (main (: n Int64))
              (Set.len (Set.of (list (Full (Full n)) (Full (Full 5)) (Full (Nil unit))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2 Int64))
  (call   main (: 6 Int64)) (output (: 3 Int64)))
