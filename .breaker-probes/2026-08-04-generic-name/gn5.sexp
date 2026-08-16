(case "gn5 a bare generic name without its argument gets the constructor-needs-arg reject"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: b Box)) 0)
        (export main)))
  (error  CDZ0203))
