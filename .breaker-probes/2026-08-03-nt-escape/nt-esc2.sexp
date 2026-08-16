(case "nt-esc2 exact fix-note render claim"
  (input  (do
            (type W (Mk Int64))
            (def (main (: k Int64)) (Mk k))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 W)))
