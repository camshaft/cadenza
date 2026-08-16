(do
  (effect S (op add (-> Int64 Int64)) (op pick (-> Int64 Int64)))
  (def (walk (: k Int64))
    (if (< k 1) 0 (let ((_d (S.add k))) (walk (- k 1)))))
  (def (main (: n Int64))
    (handle S ""
      ((add (v) s (resume 0 (String.concat s "z")))
       (pick (i) s
        (resume (match (String.at s i)
                  ((Some c) (String.byte-len c))
                  ((None _u) -1))
                s)))
      (let ((_w (walk n)))
        (S.pick (- n 1)))))
  (export main))
