(case "hh2 the helper's handle SHADOWS the same effect: inner B-handle inside an outer B-handle's body"
  (input  (do
            (effect B (op b (-> Unit Int64)))
            (def (inner-unit (: n Int64))
              (handle B (* n 10)
                ((b (u) t (resume t t)))
                (B.b)))
            (def (main (: k Int64))
              (handle B k
                ((b (u) t (resume t t)))
                (+ (B.b) (inner-unit 7))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 75 Int64)))
