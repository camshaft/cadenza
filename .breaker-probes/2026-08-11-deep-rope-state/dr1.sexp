(case "dr1 a String state grown by 200 recursive dispatches — the deep rope's byte-len and boundary slices stay exact"
  (input  (do
            (effect S (op add (-> Int64)) (op done (-> Int64)))
            (def (walk (: k Int64))
              (if (< k 1) 0 (let ((_d (S.add))) (walk (- k 1)))))
            (def (main (: n Int64))
              (handle S "x"
                ((add () s (resume (String.byte-len s) (String.concat s "ab")))
                 (done () s (resume (String.byte-len s) s)))
                (let ((_w (walk n)))
                  (S.done))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 401 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
