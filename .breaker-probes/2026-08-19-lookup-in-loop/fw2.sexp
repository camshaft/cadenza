(case "fw2 an INCREMENTAL rebuild walks 40 keys transforming each value (map-over-map by lookup)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 2)))))
            (def (xform (: i Int64) (: src (Map Int64 Int64)) (: dst (Map Int64 Int64)))
              (if (= i 0) dst
                (xform (- i 1) src
                  (match (Map.lookup src i)
                    ((Some v) (Map.insert dst i (+ v 1)))
                    ((None _u) dst)))))
            (def (main (: n Int64))
              (do
                (def src (fill n Map.empty))
                (def dst (xform n src Map.empty))
                (+ (* 10 (Map.len dst))
                   (match (Map.lookup dst 25) ((Some v) (if (= v 51) 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 401 Int64)))
