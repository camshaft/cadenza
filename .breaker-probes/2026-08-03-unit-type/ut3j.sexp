(case "ut3j plain LIST literal Nil-first, len only"
  (input  (do
        (type (Box a) (Full a) (Nil unit))
        (def (main (: k Int64))
          (List.len (list (Nil unit) (Full 1))))
        (export main)))
  (call   main (: 4 Int64)) (output (: 2 Int64)))
