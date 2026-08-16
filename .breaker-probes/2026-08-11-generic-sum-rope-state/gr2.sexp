(case "gr2 the rope grows INSIDE the generic Full wrapper each recursive dispatch — Hole seeds, per-hop rebuild, drain reads the payload length"
  (input  (do
            (effect S (op add (-> Int64 Int64)) (op len (-> Int64)))
            (type (Box a) (Full a) (Hole))
            (def (walk (: k Int64))
              (if (< k 1) 0 (let ((_d (S.add k))) (walk (- k 1)))))
            (def (main (: n Int64))
              (handle S (Hole)
                ((add (v) st
                  (resume 0 (match st
                              ((Full s) (Full (String.concat s "z")))
                              ((Hole) (Full "z")))))
                 (len () st
                  (resume (match st ((Full s) (String.byte-len s)) ((Hole) -1)) st)))
                (let ((_w (walk n)))
                  (S.len))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64))
  (call   main (: 0 Int64)) (output (: -1 Int64)))
