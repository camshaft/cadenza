(case "nt-esc a scalar-erased newtype value ESCAPES a parameterized def (the #1542 face)"
  (input  (do
            (type W (Mk Int64))
            (def (main (: k Int64)) (Mk k))
            (export main)))
  (call   main (: 5 Int64)) (output (: (Mk 5) W)))
