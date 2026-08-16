(case "a fold of unions over overlapping windows dedups globally to the element count"
  (doc    "Union as the fold OPERATOR over overlapping operands: each iteration unions a 2-element
           window {i, i+1} whose SECOND element is the NEXT window's first — n windows contribute
           2n elements but the global set is exactly n+1 (5 → 6; 1 → 2). Every intermediate union's
           dedup must be exact (an off-by-one in ONE window's overlap dedup inflates the final len)
           and the accumulator threads through n path-copied unions. The pinned union cases are
           single-step (:847 commutativity, :2376 slotting); the interval-coverage idiom (merging
           overlapping ranges into a visited set) is the loop that composes them.")
  (input  (do
            (def (windows (: i Int64) (: n Int64) (: acc (Set Int64)))
              (if (= i n)
                acc
                (windows (+ i 1) n (Set.union acc (Set.of (list i (+ i 1)))))))
            (def (main (: n Int64))
              (Set.len (windows 0 n (Set.of (list)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64))
  (call   main (: 1 Int64)) (output (: 2 Int64)))
