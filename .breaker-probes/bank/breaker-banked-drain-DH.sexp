(case "Int64.min and Int64.max coexist as Map keys with correct lookups and enumeration order"
  (doc    "The extreme-key face: Int64.min, Int64.max, and 0 in one CHAMP — both extremes look up
           correctly (1000s: min↦2; 100s: max↦1) and Map.to-list enumerates SIGNED (min first, max
           last: 10s reads the first entry's value 2, 1s the last entry's 1) → 2121. min's bit
           pattern is the all-but-top-zero hash edge and the value a signed/unsigned confusion
           displaces to the LARGEST position (enumeration would read 3..1 or 1..2 wrong); max is its
           complement. The extreme companion of the signed-key enumeration pin (CG's -k/0/5 grid
           never touches the representable boundary).")
  (input  (do
            (def (main (: a Int64))
              (let ((m (Map.insert (Map.insert (Map.insert Map.empty
                          9223372036854775807 1)
                          -9223372036854775808 2)
                          a 3)))
                (+ (* 1000 (match (Map.lookup m -9223372036854775808) ((Some v) v) ((None u) -1)))
                   (+ (* 100 (match (Map.lookup m 9223372036854775807) ((Some v) v) ((None u) -1)))
                      (+ (* 10 (. (Option.expect (List.at (Map.to-list m) 0) "f") 1))
                         (. (Option.expect (List.at (Map.to-list m) 2) "l") 1))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2121 Int64)))
