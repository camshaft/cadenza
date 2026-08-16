(case "fp3 a runtime-value bits CONSTRUCTION decoded back by a bits PATTERN (cross-byte round-trip)"
  (input  (do (def (run (: x Int64) (: y Int64))
                (match (bin (bits ((. (UInt 6) wrap) x) 6) (bits ((. (UInt 10) wrap) y) 10))
                  ((bin (bits a 6) (bits b 10)) (+ (* 10000 a) b))
                  (_other -1)))
              (export run)))
  (call   run (: 41 Int64) (: 733 Int64))
  (output (: 410733 Int64)))
