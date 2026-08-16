(case "ae3 control: performing-cond, ONE branch aborts, other returns a value"
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op hi (-> Unit Int64)))
            (def (main (: n Int64))
              (+ (handle B 0
                   ((hi (u) t 200))
                   (handle A n
                     ((tick (u) s (resume s (+ s 1))))
                     (if (> (A.tick) 3) (B.hi) -1)))
                 1))
            (export main)))
  (call   main (: 5 Int64)) (output (: 201 Int64)))
