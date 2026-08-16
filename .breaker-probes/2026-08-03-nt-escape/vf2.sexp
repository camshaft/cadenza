(case "vf2 nullary compound-newtype control: what does the const path render"
  (input  (do
            (type P (P (Tuple Int64 Int64)))
            (def (main) (P (tuple 5 6)))
            (export main)))
  (call   main) (output (: (tuple 5 6) P)))
