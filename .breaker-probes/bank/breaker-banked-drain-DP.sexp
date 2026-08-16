(case "sums with ROPE-String payloads dedup in a Set by content through the variant walk"
  (doc    "The heap-payload face of sum-element Set identity (the pinned family covers scalar payloads
           :171 and recursive spines :154 — no HEAP payload inside the variant): `(Msg \"a\"+\"b\")`
           and `(Msg \"ab\")` must collapse to ONE element (the per-element hash/compare descends
           through the variant tag INTO the rope and canonicalizes it — a chunk-shape hash keeps
           both), while `(Msg \"cd\")` and `(Nil)` stay distinct → len 3; and membership finds the
           rope-built `(Msg \"c\"+\"d\")` probe against the flat-stored twin (contains → 1) → 31.
           The Set companion of the sum-rope-payload ORDER pin (t813 family).")
  (input  (do
            (type Ev (Msg String) (Nil))
            (def (main (: k Int64))
              (let ((s (Set.of (list (Msg (String.concat "a" "b"))
                                     (Msg "ab")
                                     (Msg "cd")
                                     (Nil)))))
                (+ (* 10 (Set.len s))
                   (if (Set.contains s (Msg (String.concat "c" "d"))) 1 0))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 31 Int64)))
