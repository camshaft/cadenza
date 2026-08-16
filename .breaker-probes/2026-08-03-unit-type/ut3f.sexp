(case "ut3f Both-match plus Neither-set, ONE param sum"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: k Int64))
          (+ (* 10 (match (Full k) ((Full n) n) ((Nil _u) -1)))
             (Set.len (Set.of (list (Nil unit) (Nil unit))))))
        (export main)))
  (call   main (: 4 Int64)) (output (: 41 Int64)))
