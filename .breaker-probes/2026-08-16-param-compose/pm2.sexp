(case "pm2 a @param value seeding a HEAP structure (param-driven config list)"
  (input  (do
            (pragma param (param (: widget slider)) (: size Int64))
            (def (fill (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc (fill (- i 1) (List.push acc i))))
            (def (main)
              (host (Param)
                (do
                  (def n (Param.size))
                  (def xs (fill n (list)))
                  (+ (* 10 (List.len xs))
                     (match (List.at xs 0) ((Some v) v) ((None _u) -1))))))
            (export main)))
  (call   main)
  (host-responses (respond Param.size (: 6 Int64)))
  (output (: 66 Int64)))
