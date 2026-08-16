(case "gn2 a generic named in ANOTHER generic's payload type (nested by-name reference)"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (type (Wrap a) (Mk (Box a)))
        (def (main (: k Int64))
          (match (Mk (Full k)) ((Mk inner) (match inner ((Full v) v) ((Nil _u) -1)))))
        (export main)))
  (call   main (: 9 Int64)) (output (: 9 Int64)))
