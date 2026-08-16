(case "se3 a DRAIN-style arm removes on hit — the second take of the same key routes to the miss path"
  (input  (do
            (effect Sx (op take (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Sx (Set.of (list 1 2 3))
                ((take (v) s (if (Set.contains s v)
                                 (resume (Set.len (Set.remove s v)) (Set.remove s v))
                                 (resume (* 100 (Set.len s)) s))))
                (+ (Sx.take n) (+ (* 10 (Sx.take n)) (Sx.take 2)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 2003 Int64))
  (call   main (: 9 Int64)) (output (: 3302 Int64)))
