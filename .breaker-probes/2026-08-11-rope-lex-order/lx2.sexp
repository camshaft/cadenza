(case "lx2 PREFIX-order edges — the state string compares against crossed op-arg strings: equal, longer-prefix, and shorter-prefix faces"
  (input  (do
            (effect S (op vs (-> String Int64)))
            (def (main (: n Int64))
              (handle S (if (= (% n 2) 0) "mm" "mz")
                ((vs (probe) s (resume (if (< s probe) -1 (if (= s probe) 0 1)) s)))
                (+ (* 100 (S.vs "mm"))
                   (+ (* 10 (S.vs "m")) (S.vs "mzz")))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 9 Int64))
  (call   main (: 3 Int64)) (output (: 109 Int64)))
