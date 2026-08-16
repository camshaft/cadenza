(case "gn3 a parenthesized-head generic as a FUNCTION param annotation crossing a call"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (unbox (: b (Box Int64))) (match b ((Full v) v) ((Nil _u) -1)))
        (def (main (: k Int64)) (unbox (Full k)))
        (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
