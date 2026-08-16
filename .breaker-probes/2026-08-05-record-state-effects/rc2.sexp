(case "rc2 the record-with-heap-field state read by an ABORT arm (the sk class x record-wrapper face)"
  (input  (do
            (effect St (op push (-> Int64 Int64)) (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (record (count 0) (items (list)))
                ((push (v) s (resume (. s count)
                                     (record (count (+ (. s count) 1))
                                             (items (List.push (. s items) v)))))
                 (halt (u) s (+ (* 100 (. s count)) (List.len (. s items)))))
                (do
                  (def c1 (St.push a))
                  (def c2 (St.push (+ a 1)))
                  (St.halt))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 202 Int64)))
