(case "oi3 a LIST of Nil-first sets — the grounding member in a SIBLING element's set does not leak"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: k Int64))
          (+ (Set.len (Set.of (list (Nil unit) (Full k))))
             (* 10 (Set.len (Set.of (list (Full (* k 2)) (Nil unit)))))))
        (export main)))
  (call   main (: 4 Int64)) (output (: 22 Int64)))
