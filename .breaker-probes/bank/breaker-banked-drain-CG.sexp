(case "Map.to-list over SIGNED Int64 keys orders negatives before positives"
  (doc    "The SIGNED face of canonical enumeration: keys {5↦1, -k↦2, 0↦3} at k=7 must enumerate
           -7, 0, 5 (values 2,3,1 → 231). The existing to-list order pins use non-negative or
           Rational keys — nothing pins a NEGATIVE Int64 key's position, and an enumeration that
           ordered by the key's canonical BYTE form or unsigned value would sort -7 (top bit set)
           AFTER the positives (→ 312/132), the exact signed-vs-unsigned confusion the u64-carrier
           trap family produces elsewhere. Runtime k defeats folding.")
  (input  (do
            (def (main (: k Int64))
              (let ((m (Map.insert (Map.insert (Map.insert Map.empty 5 1) (- 0 k) 2) 0 3)))
                (+ (* 100 (. (Option.expect (List.at (Map.to-list m) 0) "a") 1))
                   (+ (* 10 (. (Option.expect (List.at (Map.to-list m) 1) "b") 1))
                      (. (Option.expect (List.at (Map.to-list m) 2) "c") 1)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 231 Int64)))
