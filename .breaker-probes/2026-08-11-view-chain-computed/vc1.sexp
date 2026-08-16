(case "vc1 a computed SLICE of the effect-grown rope then a computed String.at INTO the slice — chained view reads post-#18"
  (input  (do
            (effect S (op add (-> Int64 Int64)) (op win (-> Int64 Int64)))
            (def (walk (: k Int64))
              (if (< k 1) 0 (let ((_d (S.add k))) (walk (- k 1)))))
            (def (main (: n Int64))
              (handle S ""
                ((add (v) s (resume 0 (String.concat s "xy")))
                 (win (i) s
                  (resume (match (String.slice s i (+ i 3))
                            ((Some w) (match (String.at w (- i i))
                                        ((Some c) (String.byte-len c))
                                        ((None _u) -5)))
                            ((None _u) -1))
                          s)))
                (let ((_w (walk n)))
                  (S.win (- n 1)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1 Int64))
  (call   main (: 1 Int64)) (output (: -1 Int64)))
