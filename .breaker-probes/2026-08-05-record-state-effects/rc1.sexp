(case "rc1 a RECORD handler state with a heap FIELD (List) updated per perform, read by a later observer"
  (input  (do
            (effect St (op push (-> Int64 Int64)) (op stats (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (record (count 0) (items (list)))
                ((push (v) s (resume (. s count)
                                     (record (count (+ (. s count) 1))
                                             (items (List.push (. s items) v)))))
                 (stats (u) s (resume (+ (* 10 (. s count)) (List.len (. s items))) s)))
                (do
                  (def c1 (St.push a))
                  (def c2 (St.push (+ a 1)))
                  (+ (* 100 c2) (St.stats)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 122 Int64)))
