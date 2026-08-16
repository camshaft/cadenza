(case "a Set drains through to-list head extraction in canonical order like the Map twin"
  (doc    "The Set companion of the Map priority-drain pin: each iteration extracts the canonical
           minimum (`List.at (Set.to-list s) 0`), removes it, folds positionally — the digit string
           IS the drain order (k=2 → 259; k=7 → 579). Composes canonical Set enumeration with
           remove-path canonicalization in the loop that matters (the shrunk set's NEXT minimum must
           be right after every removal — a non-canonical remove surfaces as a wrong middle digit).
           With the Map twin this pins the min-extract loop over BOTH ordered-enumeration CHAMPs.")
  (input  (do
            (def (drain (: s (Set Int64)) (: acc Int64) (: fuel Int64))
              (if (= fuel 0)
                acc
                (if (= (Set.len s) 0)
                  acc
                  (match (List.at (Set.to-list s) 0)
                    ((Some e) (drain (Set.remove s e) (+ (* acc 10) e) (- fuel 1)))
                    ((None u) acc)))))
            (def (main (: k Int64))
              (drain (Set.of (list 5 k 9)) 0 10))
            (export main)))
  (call   main (: 2 Int64)) (output (: 259 Int64))
  (call   main (: 7 Int64)) (output (: 579 Int64)))
