(case "zx1 a ZIP walk over two enumerations: paired to-lists fold together by position"
  (input  (do
            (def (filla (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (filla (- i 1) (Map.insert m i (* i 2)))))
            (def (fillb (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fillb (- i 1) (Map.insert m i (* i 3)))))
            (def (zipsum (: xs (List (Tuple Int64 Int64))) (: ys (List (Tuple Int64 Int64))) (: acc Int64))
              (match xs
                ((list) acc)
                ((list xh .. xt)
                  (match ys
                    ((list) acc)
                    ((list yh .. yt)
                      (match xh ((tuple _xk xv)
                        (match yh ((tuple _yk yv)
                          (zipsum xt yt (+ acc (* xv yv))))))))))))
            (def (main (: n Int64))
              (do
                (def a (filla n Map.empty))
                (def b (fillb n Map.empty))
                (zipsum (Map.to-list a) (Map.to-list b) 0)))
            (export main)))
  (call   main (: 20 Int64)) (output (: 17220 Int64)))
