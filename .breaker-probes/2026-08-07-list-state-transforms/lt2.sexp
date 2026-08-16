(case "lt2 the arm DOUBLES every list element per dispatch (via a projection helper) — the sum geometrically grows"
  (input  (do
            (effect L (op amp (-> Int64)))
            (def (el (: s (List Int64)) (: i Int64))
              (match (List.at s i) ((Some v) v) ((None) 0)))
            (def (main (: n Int64))
              (handle L (list n 3)
                ((amp () s (resume (+ (el s 0) (el s 1))
                                   (List.update (List.update s 0 (* (el s 0) 2)) 1 (* (el s 1) 2)))))
                (+ (L.amp) (+ (* 10 (L.amp)) (* 100 (L.amp))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3368 Int64))
  (call   main (: 0 Int64)) (output (: 1263 Int64)))
