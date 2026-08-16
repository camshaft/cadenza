(case "gn1 a parenthesized-head generic resolves by name in an annotation and runs"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: k Int64))
          (let (((: b (Box Int64)) (Full k)))
            (match b ((Full v) v) ((Nil _u) -1))))
        (export main)))
  (call   main (: 7 Int64)) (output (: 7 Int64)))
