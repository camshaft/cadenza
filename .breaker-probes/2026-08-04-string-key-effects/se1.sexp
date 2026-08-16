(case "se1 ROPE-built String keys inserted into a Map handler state across performs; probe by FLAT twin"
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "y") (- n 1))))
            (effect Acc (op put (-> Int64 Int64)))
            (def (main (: a Int64) (: b Int64))
              (handle Acc Map.empty
                ((put (v) s (resume (Map.len s) (Map.insert s (String.concat "k" (rep "" v)) v))))
                (do
                  (def l1 (Acc.put a))
                  (def l2 (Acc.put b))
                  (+ (* 100 l1) (+ (* 10 l2) (Acc.put 3))))))
            (export main)))
  (call   main (: 1 Int64) (: 2 Int64))
  (output (: 12 Int64)))
