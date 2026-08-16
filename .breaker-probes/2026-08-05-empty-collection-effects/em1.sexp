(case "em1 EMPTY collections as op args/resume values: len/contains on effect-crossed empties"
  (input  (do
            (effect St (op mk (-> Unit (List Int64))) (op count (-> (List Int64) Int64)))
            (def (main (: n Int64))
              (handle St n
                ((mk (u) s (resume (list) s))
                 (count (xs) s (resume (List.len xs) s)))
                (+ (* 10 (List.len (St.mk)))
                   (St.count (list)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 0 Int64)))
