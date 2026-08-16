(case "a Set partitioned by a predicate into two Sets preserves membership and counts"
  (doc    "The dual-accumulator partition (split a Set into two by a threshold): fold Set.to-list,
           routing each element to the `lo` or `hi` accumulator set by `< t` — TWO growing CHAMPs
           threaded through one recursion. Runtime k lands in hi at k=7 (lo {1,2}, hi {5,7,8} → 231)
           or lo at k=3 (lo {1,2,3}, hi {5,8} → 320); the k-membership digit confirms it routed to
           the right set. A partition that shared node structure between the two accumulators (a
           misrouted insert lands in both, or one accumulator's path-copy corrupts the other) breaks
           a count; the two independent result CHAMPs threading through the fold is the seam. The
           GROUP-BY / bucketing idiom.")
  (input  (do
            (def (part (: es (List Int64)) (: lo (Set Int64)) (: hi (Set Int64)) (: t Int64))
              (match es
                ((list) (tuple lo hi))
                ((list e .. rest)
                  (if (< e t) (part rest (Set.insert lo e) hi t)
                              (part rest lo (Set.insert hi e) t)))))
            (def (main (: k Int64))
              (let ((s (Set.of (list 1 5 k 8 2))))
                (match (part (Set.to-list s) (Set.of (list)) (Set.of (list)) 5)
                  ((tuple lo hi)
                    (+ (* 100 (Set.len lo))
                       (+ (* 10 (Set.len hi))
                          (if (Set.contains hi k) 1 0)))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 231 Int64))
  (call   main (: 3 Int64)) (output (: 320 Int64)))
