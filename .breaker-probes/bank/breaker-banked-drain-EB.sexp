(case "fused-match arms each build a DIFFERENT rope and the joined String result reads correctly"
  (doc    "The heap-RESULT face of the fused-clone seam: each arm of a match on a CALL result builds
           its own runtime rope (\"big\"+\"ger\" / \"sm\"+\"all\") and the match's value joins into
           ONE String slot read by byte-len AND content-eq — k=7 → \"bigger\" (6 bytes, eq hits) →
           61; k=2 → \"small\" (5 bytes, eq misses) → 50. The fused clones materialize DIFFERENT
           heap allocations into the same join slot; a join that specialized the slot to one arm's
           allocation shape (or freed the untaken arm's constant chunks early) breaks a call. The
           heap-alloc companion of the scalar fused-arm pins (BG family reads scalars out of arms).")
  (input  (do
            (type Sz (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (def (main (: k Int64))
              (let ((s (match (mk k)
                         ((Hi h) (String.concat "big" "ger"))
                         ((Lo w) (String.concat "sm" "all")))))
                (+ (* 10 (String.byte-len s))
                   (if (= s "bigger") 1 0))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 61 Int64))
  (call   main (: 2 Int64)) (output (: 50 Int64)))
