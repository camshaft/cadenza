(case "gn7 param-annotation mismatch rejects (unbox expects Box String, gets Full Int64)"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (unbox (: b (Box String))) (match b ((Full _v) 1) ((Nil _u) -1)))
        (def (main (: k Int64)) (unbox (Full k)))
        (export main)))
  (error  CDZ0203))
