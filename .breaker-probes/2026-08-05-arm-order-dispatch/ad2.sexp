(case "ad2 FIVE ops on one effect, dispatch checksum across all arms (wide handler routing)"
  (input  (do
            (effect St (op a (-> Unit Int64)) (op b (-> Unit Int64)) (op c (-> Unit Int64)) (op d (-> Unit Int64)) (op e (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume 1 s))
                 (b (u) s (resume 2 s))
                 (c (u) s (resume 3 s))
                 (d (u) s (resume 4 s))
                 (e (u) s (resume 5 s)))
                (+ (* 10000 (St.e)) (+ (* 1000 (St.a)) (+ (* 100 (St.d)) (+ (* 10 (St.b)) (St.c)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 51423 Int64)))
