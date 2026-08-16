(case "vc2 a SLICE OF A SLICE with computed bounds over the effect-grown rope — double view depth stays exact"
  (input  (do
            (effect S (op add (-> Int64 Int64)) (op deep (-> Int64 Int64)))
            (def (walk (: k Int64))
              (if (< k 1) 0 (let ((_d (S.add k))) (walk (- k 1)))))
            (def (main (: n Int64))
              (handle S ""
                ((add (v) s (resume 0 (String.concat s "abc")))
                 (deep (i) s
                  (resume (match (String.slice s i (+ i 6))
                            ((Some w1) (match (String.slice w1 1 (+ i 2))
                                         ((Some w2) (String.byte-len w2))
                                         ((None _u) -6)))
                            ((None _u) -1))
                          s)))
                (let ((_w (walk n)))
                  (S.deep (- n 2)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 2 Int64))
  (call   main (: 4 Int64)) (output (: 3 Int64)))
