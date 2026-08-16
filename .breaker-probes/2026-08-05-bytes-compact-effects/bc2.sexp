(case "bc2 the compacted state REPLACES the rope in the next-state slot (compaction as a state transition)"
  (input  (do
            (effect St (op grow (-> Int64 Int64)) (op squash (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Bytes.of (list))
                ((grow (v) s (resume (Bytes.len s) (Bytes.concat s (Bytes.of (list (UInt8.wrap v))))))
                 (squash (u) s (resume (Bytes.len s) (Bytes.compact s))))
                (+ (* 1000 (St.grow 1))
                   (+ (* 100 (St.grow 2))
                      (+ (* 10 (St.squash)) (St.grow 3))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 122 Int64)))
