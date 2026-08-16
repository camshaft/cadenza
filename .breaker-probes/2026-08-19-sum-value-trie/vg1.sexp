(case "vg1 a MATCH over a trie-retrieved SUM value dispatches variants from depth"
  (input  (do
            (type Ev (Add Int64) (Del Int64) (Noop))
            (def (fill (: i Int64) (: m (Map Int64 Ev)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m i
                  (if (= (% i 3) 0) (Ev.Noop)
                    (if (= (% i 3) 1) (Ev.Add (* i 2)) (Ev.Del i)))))))
            (def (classify (: m (Map Int64 Ev)) (: k Int64))
              (match (Map.lookup m k)
                ((Some e) (match e
                            ((Ev.Add v) v)
                            ((Ev.Del v) (- 0 v))
                            ((Ev.Noop) 0)))
                ((None _u) -99999)))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 1000 (classify m 10))
                   (+ (* 10 (classify m 11))
                      (classify m 12)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 19890 Int64)))
