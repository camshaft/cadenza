(case "Ordering values as Map KEYS dispatch by discriminant with all three variants"
  (doc    "The BUILTIN-nullary-sum-as-key face: a 3-entry map keyed by Less/Equal/Greater, probed by
           a runtime `compare` result — each of the three comparisons routes to its own entry (1/2/3).
           The key hash/eq must read the DISCRIMINANT of the unit-payload builtin sum (the #42/#43
           order-bug family showed builtin sums get special reps per backend — this pins their
           CONTENT identity at the CHAMP boundary where a rep-keyed hash would split or collide the
           variants). The dispatch-on-comparison-outcome idiom (three-way branch tables).")
  (input  (do
            (def (main (: a Int64) (: b Int64))
              (let ((m (Map.insert (Map.insert (Map.insert Map.empty
                          (Ordering.Less unit) 1)
                          (Ordering.Equal unit) 2)
                          (Ordering.Greater unit) 3)))
                (match (Map.lookup m (compare a b))
                  ((Some v) v) ((None u) -1))))
            (export main)))
  (call   main (: 3 Int64) (: 7 Int64)) (output (: 1 Int64))
  (call   main (: 7 Int64) (: 7 Int64)) (output (: 2 Int64))
  (call   main (: 9 Int64) (: 7 Int64)) (output (: 3 Int64)))
