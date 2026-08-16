(case "g5 SOME-payload guard over a string (heap payload, not bare binder)"
  (input  (do
            (def (band (: o (Option String)))
              (match o ((guard (Some t) (< t "m")) 1) (_ 3)))
            (def (main (: k Int64)) (band (Some "apple")))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
