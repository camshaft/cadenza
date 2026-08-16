(case "fx5 boundary values ROUND-TRIP through a host-visible sum payload and compare"
  (input  (do
            (type (Box a) (Full a) (Nil unit))
            (def (main (: k Int64))
              (match (Full (+ 536870911 k))
                ((Full v) (if (= v 536870912) 1 0))
                ((Nil _u) -1)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64)))
