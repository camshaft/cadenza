(case "gs2 a TWO-parameter generic sum by name — (Pair Int64 Int64) in the annotation, both payload slots extracted"
  (input  (do
            (type (Pair a b) (Both a b))
            (def (mix (: p (Pair Int64 Int64))) (match p ((Both x y) (+ (* 10 x) y))))
            (def (main (: k Int64)) (mix (Both k 3)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 73 Int64))
  (call   main (: -2 Int64)) (output (: -17 Int64)))
