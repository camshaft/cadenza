(case "gn4 a HEAD-ONLY phantom param generic constructs at two instantiations and stays distinct"
  (input  (do
        (type (P a) (Mk Int64))
        (def (main (: k Int64))
          (match (Mk k) ((Mk v) v)))
        (export main)))
  (call   main (: 6 Int64)) (output (: 6 Int64)))
