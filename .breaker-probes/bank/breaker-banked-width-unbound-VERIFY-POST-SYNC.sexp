(case "an unbound name in a width position rejects CDZ0101 through an annotation"
  (input  (do
            (def (main (: x (Int nosuchwidth))) x)
            (export main)))
  (call   main (: 1 Int64))
  (error  CDZ0101))
