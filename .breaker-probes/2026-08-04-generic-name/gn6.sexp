(case "gn6 by-name annotation at a MISMATCHED instantiation rejects (Box String vs Full Int)"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: k Int64))
          (let (((: b (Box String)) (Full k)))
            (match b ((Full _v) 1) ((Nil _u) -1))))
        (export main)))
  (error  CDZ0203))
