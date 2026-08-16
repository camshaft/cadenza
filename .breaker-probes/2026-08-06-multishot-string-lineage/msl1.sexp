(case "msl1 divergent multi-shot STRING lineages — each branch observes its own byte-length"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)) (op len (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb ""
                ((flip (u) s (+ (resume 1 (String.concat s "a")) (resume 2 (String.concat s "bb"))))
                 (len (u) s (resume (String.byte-len s) s)))
                (+ (* 10 (Amb.flip)) (Amb.len))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 33 Int64)))
