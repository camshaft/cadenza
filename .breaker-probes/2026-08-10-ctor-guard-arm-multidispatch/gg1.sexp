(case "gg1 a GUARD over the unwrapped generic payload — (guard (Full v) (> v s)) admits high-water marks, the reject path leaves the state"
  (input  (do
            (type (Container a) (Full a))
            (effect E (op rate (-> (Container Int64) Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((rate (c) s
                  (match c
                    ((guard (Full v) (> v s)) (resume v v))
                    ((Full _v) (resume 0 s)))))
                (+ (* 10 (E.rate (Full k))) (E.rate (Full 3)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64))
  (call   main (: 1 Int64)) (output (: 13 Int64))
  (call   main (: -2 Int64)) (output (: 3 Int64)))
