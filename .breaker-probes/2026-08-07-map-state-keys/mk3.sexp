(case "mk3 the arm SHRINKS the map state via remove — re-removing the same key is idempotent, a missing key is a no-op"
  (input  (do
            (effect Reg (op drop (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Reg (map (1 11) (2 22) (3 33))
                ((drop (k) s (resume (Map.len (Map.remove s k)) (Map.remove s k))))
                (+ (Reg.drop n) (+ (* 10 (Reg.drop n)) (* 100 (Reg.drop 3))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 122 Int64))
  (call   main (: 9 Int64)) (output (: 233 Int64)))
