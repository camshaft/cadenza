(case "tt4 a MIXED String+Int tuple state — the arm grows the rope and folds the op arg into the counter per dispatch"
  (input  (do
            (effect E (op log (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (tuple "go" n)
                ((log (v) s (match s
                              ((tuple w k) (resume (+ (String.byte-len w) k)
                                                   (tuple (String.concat w "!") (+ k v)))))))
                (+ (E.log 100) (+ (* 10 (E.log 0)) (* 100 (E.log 5))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11987 Int64))
  (call   main (: 0 Int64)) (output (: 11432 Int64)))
