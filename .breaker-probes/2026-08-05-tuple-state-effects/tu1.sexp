(case "tu1 a TUPLE handler state with mixed scalar+heap components advanced independently per op"
  (input  (do
            (effect St (op inc (-> Unit Int64)) (op push (-> Int64 Int64)) (op read (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (tuple 0 (list))
                ((inc (u) s (resume 0 (tuple (+ (. s 0) 1) (. s 1))))
                 (push (v) s (resume 0 (tuple (. s 0) (List.push (. s 1) v))))
                 (read (u) s (resume (+ (* 10 (. s 0)) (List.len (. s 1))) s)))
                (do
                  (def _a (St.inc))
                  (def _b (St.push a))
                  (def _c (St.inc))
                  (def _d (St.push (+ a 1)))
                  (St.read))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 22 Int64)))
