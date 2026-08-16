(case "a two-level List.update replaces an inner element and leaves the original grid persistent"
  (doc    "The nested-List UPDATE (the two-level INDEX pin :3231 only reads; the map-value two-level
           update :16969 is a Map): `List.update grid 1 (List.update row 0 99)` rebuilds the outer
           RRB with a NEW inner row whose element 0 is replaced — grid2[1][0] = 99 (1000s), the
           SIBLING row grid2[0] is untouched (10s: 11), and the ORIGINAL grid still reads its
           pre-update inner (1s: grid[1][0] = 20, so 20-(n-1) = 0 at n=21) → 99110. Both RRB levels
           path-copy independently; an in-place inner update corrupts the original, and a sibling-row
           aliasing bug flips the 10s digit. The nested-List persistence face the index pin can't
           reach.")
  (input  (do
            (def (main (: n Int64))
              (let ((grid (list (list 10 11) (list 20 n 22))))
                (let ((row (Option.expect (List.at grid 1) "r")))
                  (let ((grid2 (List.update grid 1 (List.update row 0 99))))
                    (+ (* 1000 (Option.expect (List.at (Option.expect (List.at grid2 1) "r2") 0) "e2"))
                       (+ (* 10 (Option.expect (List.at (Option.expect (List.at grid2 0) "r0") 1) "sib"))
                          (- (Option.expect (List.at (Option.expect (List.at grid 1) "og") 0) "og0") (- n 1))))))))
            (export main)))
  (call   main (: 21 Int64)) (output (: 99110 Int64)))
