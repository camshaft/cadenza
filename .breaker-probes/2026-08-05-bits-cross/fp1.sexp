(case "fp1 a bits run spanning TWO bytes (3+13) decodes MSB-first over a runtime scrutinee"
  (input  (do (def (run (: h Int64))
                (match (Bytes.of (list (UInt8.wrap h) (UInt8.wrap 90)))
                  ((bin (bits a 3) (bits b 13)) (+ (* 100000 a) b))
                  (_other -1)))
              (export run)))
  (call   run (: 182 Int64))
  (output (: 505722 Int64)))
