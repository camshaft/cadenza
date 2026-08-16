(case "sc1 a THREE-link seed CHAIN of DISTINCT effects — each handle's whole region value seeds the next"
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op get (-> Unit Int64)))
            (effect C (op h (-> Unit Int64)))
            (def (main (: n Int64))
              (handle C
                (handle B
                  (handle A n ((tick (u) s (resume s (+ s 2))))
                    (+ (A.tick) (* 10 (A.tick))))
                  ((get (u) t (resume t (+ t 1))))
                  (+ (B.get) (* 10 (B.get))))
                ((h (u) w (resume w (+ w 5))))
                (+ (C.h) (C.h))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1675 Int64))
  (call   main (: 0 Int64)) (output (: 465 Int64))
  (call   main (: -6 Int64)) (output (: -987 Int64)))
